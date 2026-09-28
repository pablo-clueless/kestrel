//! Serves the built UI (`kestrel/out`, a Next.js static export) from the engine, so one binary or
//! container is the whole app. HANDOFF → M5.
//!
//! Release builds embed the files at compile time; debug builds read them from disk, so a
//! `pnpm build` shows up without recompiling the engine. If the UI hasn't been built, the engine
//! still compiles and runs; pages explain how to build it.

use axum::{
    body::Body,
    extract::State,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

use super::AppState;

#[derive(Embed)]
#[folder = "../kestrel/out/"]
#[allow_missing = true]
struct Ui;

const NOT_BUILT: &str = "The UI isn't built into this engine. Run `pnpm build` in kestrel/, then restart \
                         (debug builds) or rebuild (release builds).";

pub async fn serve(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Next's static export writes `/foo` as `foo.html` or `foo/index.html`.
    let candidates = if path.is_empty() {
        vec!["index.html".to_owned()]
    } else {
        vec![path.to_owned(), format!("{path}.html"), format!("{path}/index.html")]
    };

    let (status, name, file) = match candidates.iter().find_map(|c| Ui::get(c).map(|f| (c.clone(), f))) {
        Some((name, file)) => (StatusCode::OK, name, file),
        None => match Ui::get("404.html") {
            Some(file) => (StatusCode::NOT_FOUND, "404.html".to_owned(), file),
            None if Ui::get("index.html").is_none() => return (StatusCode::NOT_FOUND, NOT_BUILT).into_response(),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };

    let mime = file.metadata.mimetype().to_owned();
    let body = if mime.starts_with("text/html") {
        Body::from(inject_token(&String::from_utf8_lossy(&file.data), &state.config.token))
    } else {
        Body::from(file.data.into_owned())
    };

    let mut res = (status, body).into_response();
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    // Hashed build assets never change; pages carry the session token, so never cache them.
    let cache = if name.starts_with("_next/static/") { "public, max-age=31536000, immutable" } else { "no-store" };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

/// The UI reads the session token from `<meta name="kestrel-token">` when the engine serves it, so
/// nothing secret is baked into the build. Only reachable where the API itself is: loopback, or a
/// private network (HANDOFF → Safety rails).
fn inject_token(html: &str, token: &str) -> String {
    let meta = format!(r#"<meta name="kestrel-token" content="{}">"#, escape_attr(token));
    match html.find("</head>") {
        Some(i) => format!("{}{meta}{}", &html[..i], &html[i..]),
        None => format!("{meta}{html}"),
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_the_token_into_head_escaped() {
        let html = "<html><head><title>K</title></head><body></body></html>";
        let out = inject_token(html, r#"a"b<c"#);
        assert!(out.contains(r#"<meta name="kestrel-token" content="a&quot;b&lt;c"></head>"#), "{out}");
        assert!(inject_token("<p>no head</p>", "t").starts_with(r#"<meta name="kestrel-token" content="t">"#));
    }
}
