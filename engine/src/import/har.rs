//! HAR (HTTP Archive, saved from browser dev tools or a proxy) → collection. HANDOFF → Inputs.
//!
//! A browser HAR holds every request the page made, so only API calls are kept: assets (scripts,
//! styles, images, fonts, media), CORS preflights and non-HTTP URLs are skipped, and a request
//! repeated with the same method, URL and body (polling) is kept once. Headers the browser or the
//! HTTP client sets on its own (`:authority`, `sec-fetch-*`, `content-length`…) are dropped. The most
//! common origin becomes `base`; when there are several hosts, endpoints are grouped by host.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;
use url::Url;
use uuid::Uuid;

use super::{
    ImportResult, METHODS,
    curl::{auth_from_header, endpoint_name, is_form, kv, origin_of},
};
use crate::model::{Auth, Body, Collection, Endpoint, FieldKind, FormField, KeyValue};

/// Whether `root` is a HAR file.
pub fn is_har(root: &Value) -> bool {
    root.get("log").and_then(|l| l.get("entries")).is_some_and(Value::is_array)
}

/// File extensions of page assets, for HARs without Chrome's `_resourceType`.
const ASSET_EXTENSIONS: &[&str] = &[
    "js", "mjs", "css", "map", "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "ico", "bmp", "woff", "woff2",
    "ttf", "otf", "eot", "mp4", "webm", "mp3", "wav", "ogg", "wasm",
];

/// Response types of page assets, for HARs without Chrome's `_resourceType`.
const ASSET_TYPES: &[&str] =
    &["image/", "font/", "audio/", "video/", "text/css", "javascript", "application/font", "application/wasm"];

pub fn import(root: &Value, name: Option<&str>) -> Result<ImportResult, String> {
    let log = &root["log"];
    let entries = log["entries"].as_array().map(Vec::as_slice).unwrap_or_default();
    if entries.is_empty() {
        return Err("the HAR file has no requests".into());
    }

    let mut ctx = Ctx::default();
    let mut seen = HashSet::new();
    let mut endpoints = Vec::new();
    for entry in entries {
        if let Some(endpoint) = ctx.entry(entry, &mut seen) {
            endpoints.push(endpoint);
        }
    }

    let mut skipped = Vec::new();
    for (count, what) in [
        (ctx.assets, "page assets (scripts, styles, images, fonts)"),
        (ctx.duplicates, "repeated requests"),
        (ctx.preflights, "CORS preflights"),
        (ctx.other_schemes, "non-HTTP URLs (data:, blob:, ws:…)"),
    ] {
        if count > 0 {
            skipped.push(format!("{what}: {count}"));
        }
    }
    if endpoints.is_empty() {
        let why = if skipped.is_empty() { String::new() } else { format!(" (skipped {})", skipped.join("; ")) };
        return Err(format!("the HAR file has no API requests to import{why}"));
    }
    let mut warnings = Vec::new();
    if !skipped.is_empty() {
        warnings.push(format!("Skipped {}.", skipped.join("; ")));
    }
    warnings.append(&mut ctx.warnings);
    if ctx.credentials {
        warnings.push(
            "Some requests carry credentials (Authorization, cookies or API keys), which are saved in the \
             collection as plain text. Consider moving them into an environment secret and using {{name}} instead."
                .into(),
        );
    }

    // The most common origin (the first seen, on a tie) becomes `base`.
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut order = Vec::new();
    for origin in endpoints.iter().filter_map(|e| origin_of(&e.url)) {
        let n = counts.entry(origin.clone()).or_default();
        if *n == 0 {
            order.push(origin);
        }
        *n += 1;
    }
    let base =
        order.iter().max_by_key(|o| (counts[*o], std::cmp::Reverse(order.iter().position(|x| x == *o)))).cloned();
    let hosts: HashSet<String> = endpoints.iter().filter_map(|e| host_of(&e.url)).collect();
    for e in &mut endpoints {
        if hosts.len() > 1 {
            e.group = host_of(&e.url);
        }
        if let Some(origin) = base.as_deref().filter(|o| origin_of(&e.url).as_deref() == Some(o)) {
            e.url = format!("{{{{base}}}}{}", &e.url[origin.len()..]);
        }
    }
    let mut vars = BTreeMap::new();
    if let Some(origin) = &base {
        vars.insert("base".to_owned(), origin.clone());
    }

    let creator = log["creator"]["name"].as_str().map(str::trim).filter(|c| !c.is_empty());
    let count = endpoints.len();
    let source = match creator {
        Some(c) => format!("HAR · {c} · {count} request{}", if count == 1 { "" } else { "s" }),
        None => format!("HAR · {count} request{}", if count == 1 { "" } else { "s" }),
    };
    let collection = Collection {
        id: Uuid::new_v4(),
        name: name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .or_else(|| base.as_deref().and_then(host_of))
            .unwrap_or_else(|| "HAR import".into()),
        vars,
        headers: Vec::new(),
        endpoints,
        groups: Vec::new(),
        source: Some(source),
        schema_defs: None,
    };
    warnings.dedup();
    Ok(ImportResult { collection, format: "HAR".into(), warnings })
}

#[derive(Default)]
struct Ctx {
    warnings: Vec<String>,
    assets: usize,
    duplicates: usize,
    preflights: usize,
    other_schemes: usize,
    credentials: bool,
}

impl Ctx {
    fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }

    /// One HAR entry as an endpoint, or `None` if it's skipped (and counted).
    fn entry(&mut self, entry: &Value, seen: &mut HashSet<(String, String, String)>) -> Option<Endpoint> {
        let request = &entry["request"];
        let raw_url = request["url"].as_str()?;
        let method_name = request["method"].as_str().unwrap_or("GET").to_ascii_uppercase();
        let Some(mut url) = Url::parse(raw_url).ok().filter(|u| matches!(u.scheme(), "http" | "https")) else {
            self.other_schemes += 1;
            return None;
        };
        let mut headers: Vec<(String, String)> = request["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| Some((h["name"].as_str()?.to_owned(), h["value"].as_str().unwrap_or("").to_owned())))
            .collect();
        let has_header =
            |headers: &[(String, String)], name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));

        let resource_type = entry["_resourceType"].as_str().map(str::to_ascii_lowercase);
        if resource_type.as_deref() == Some("preflight")
            || (method_name == "OPTIONS" && has_header(&headers, "access-control-request-method"))
        {
            self.preflights += 1;
            return None;
        }
        if is_asset(resource_type.as_deref(), entry, &url) {
            self.assets += 1;
            return None;
        }

        let post = request.get("postData").filter(|p| p.is_object());
        let body_text = post.and_then(|p| p["text"].as_str()).unwrap_or("").to_owned();
        if !seen.insert((method_name.clone(), raw_url.to_owned(), body_text)) {
            self.duplicates += 1;
            return None;
        }
        let Some(method) = METHODS.iter().find(|(m, _)| m.eq_ignore_ascii_case(&method_name)).map(|(_, m)| *m) else {
            self.warn(format!("{method_name} requests aren't supported, so they were skipped."));
            return None;
        };

        let query: Vec<KeyValue> = url.query_pairs().map(|(k, v)| kv(&k, &v)).collect();
        url.set_query(None);
        url.set_fragment(None);
        let url = url.to_string();
        let label = endpoint_name(&url);

        headers.retain(|(k, _)| !dropped_header(k));
        let body = self.body(post, &mut headers, &label);
        let mut auth = Auth::None;
        if let Some((_, value)) = headers.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            && let Some(found) = auth_from_header(value)
        {
            auth = found;
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
        }
        let secret_headers = ["authorization", "cookie", "x-api-key", "api-key", "proxy-authorization"];
        self.credentials |= !matches!(auth, Auth::None)
            || headers.iter().any(|(k, _)| secret_headers.contains(&k.to_ascii_lowercase().as_str()));

        Some(Endpoint {
            id: Uuid::new_v4(),
            name: label,
            group: None,
            method,
            url,
            headers: headers.iter().map(|(k, v)| kv(k, v)).collect(),
            query,
            body,
            auth,
            expect: None,
            extract: Vec::new(),
        })
    }

    /// The request body from `postData`. The body's own type replaces the `Content-Type` header,
    /// which the engine sets when it sends.
    fn body(&mut self, post: Option<&Value>, headers: &mut Vec<(String, String)>, label: &str) -> Body {
        let Some(post) = post else { return Body::None };
        let header_type =
            headers.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone());
        let content_type = post["mimeType"].as_str().filter(|m| !m.is_empty()).map(str::to_owned).or(header_type);
        let mime = content_type.as_deref().map(|ct| ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
        let text = post["text"].as_str().unwrap_or("");
        let params = post["params"].as_array().map(Vec::as_slice).unwrap_or_default();
        if text.is_empty() && params.is_empty() {
            return Body::None;
        }
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));

        let mime = mime.unwrap_or_default();
        if mime == "multipart/form-data" && !params.is_empty() {
            let fields = params
                .iter()
                .map(|p| {
                    let file = p["fileName"].as_str().filter(|f| !f.is_empty());
                    if let Some(file) = file {
                        self.warn(format!(
                            "{label}: the file field `{}` ({file}) needs its file attached again.",
                            p["name"].as_str().unwrap_or("")
                        ));
                    }
                    FormField {
                        key: p["name"].as_str().unwrap_or("").to_owned(),
                        kind: if file.is_some() { FieldKind::File } else { FieldKind::Text },
                        value: if file.is_some() {
                            String::new()
                        } else {
                            p["value"].as_str().unwrap_or("").to_owned()
                        },
                        file: None,
                        enabled: true,
                    }
                })
                .collect();
            return Body::Multipart { fields };
        }
        if mime == "application/x-www-form-urlencoded" {
            if is_form(text) {
                let fields = url::form_urlencoded::parse(text.as_bytes()).map(|(k, v)| kv(&k, &v)).collect();
                return Body::Form { fields };
            }
            if text.is_empty() {
                let fields = params
                    .iter()
                    .map(|p| kv(p["name"].as_str().unwrap_or(""), p["value"].as_str().unwrap_or("")))
                    .collect();
                return Body::Form { fields };
            }
        }
        let looks_json = matches!(text.trim_start().chars().next(), Some('{' | '['));
        if mime == "application/json" || mime.ends_with("+json") || (mime.is_empty() && looks_json) {
            return Body::Json { content: text.to_owned() };
        }
        Body::Raw { content_type: content_type.unwrap_or_else(|| "text/plain".into()), content: text.to_owned() }
    }
}

/// Whether the entry is a page asset rather than an API call: by Chrome's `_resourceType` when
/// there is one, otherwise by the response type or the URL's extension.
fn is_asset(resource_type: Option<&str>, entry: &Value, url: &Url) -> bool {
    if let Some(kind) = resource_type {
        return !matches!(kind, "xhr" | "fetch" | "document" | "other");
    }
    let mime = entry["response"]["content"]["mimeType"].as_str().unwrap_or("").to_ascii_lowercase();
    let extension = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .and_then(|file| file.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    ASSET_TYPES.iter().any(|t| mime.contains(t)) || extension.is_some_and(|e| ASSET_EXTENSIONS.contains(&e.as_str()))
}

/// Headers the browser or the HTTP client sets on its own, which shouldn't be replayed.
fn dropped_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(':')
        || name.starts_with("sec-ch-")
        || name.starts_with("sec-fetch-")
        || matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "connection"
                | "keep-alive"
                | "proxy-connection"
                | "transfer-encoding"
                | "te"
                | "upgrade"
                | "upgrade-insecure-requests"
                | "accept-encoding"
                | "priority"
        )
}

fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::HttpMethod;

    fn entry(method: &str, url: &str, kind: Option<&str>, headers: Value, post: Option<Value>) -> Value {
        let mut e = json!({
            "request": { "method": method, "url": url, "headers": headers },
            "response": { "status": 200, "content": { "mimeType": "application/json" } }
        });
        if let Some(kind) = kind {
            e["_resourceType"] = json!(kind);
        }
        if let Some(post) = post {
            e["request"]["postData"] = post;
        }
        e
    }

    fn har(entries: Vec<Value>) -> Value {
        json!({ "log": { "version": "1.2", "creator": { "name": "WebInspector" }, "entries": entries } })
    }

    fn header<'a>(e: &'a Endpoint, name: &str) -> Option<&'a str> {
        e.headers.iter().find(|h| h.key.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }

    #[test]
    fn keeps_api_calls_and_skips_the_rest() {
        let api = "https://api.shop.io";
        let root = har(vec![
            entry(
                "GET",
                &format!("{api}/orders?page=2&sort=new"),
                Some("fetch"),
                json!([
                    { "name": ":authority", "value": "api.shop.io" },
                    { "name": "Authorization", "value": "Bearer abc" },
                    { "name": "sec-fetch-mode", "value": "cors" },
                    { "name": "Accept", "value": "application/json" },
                    { "name": "X-Tenant", "value": "acme" }
                ]),
                None,
            ),
            // Polling: the same request again.
            entry("GET", &format!("{api}/orders?page=2&sort=new"), Some("fetch"), json!([]), None),
            entry("GET", "https://shop.io/app.js", Some("script"), json!([]), None),
            entry("OPTIONS", &format!("{api}/orders"), Some("preflight"), json!([]), None),
            entry("GET", "data:image/png;base64,AAAA", None, json!([]), None),
            entry(
                "POST",
                &format!("{api}/orders"),
                Some("xhr"),
                json!([{ "name": "Content-Type", "value": "application/json" }, { "name": "Content-Length", "value": "9" }]),
                Some(json!({ "mimeType": "application/json", "text": "{\"sku\":1}" })),
            ),
            entry(
                "POST",
                &format!("{api}/login"),
                Some("fetch"),
                json!([]),
                Some(json!({ "mimeType": "application/x-www-form-urlencoded", "text": "user=ada&pass=x%20y" })),
            ),
            entry(
                "POST",
                &format!("{api}/upload"),
                Some("fetch"),
                json!([]),
                Some(json!({ "mimeType": "multipart/form-data; boundary=x", "params": [
                    { "name": "note", "value": "hi" },
                    { "name": "file", "fileName": "a.png", "contentType": "image/png" }
                ] })),
            ),
            // Another host, and a HAR without `_resourceType` (Firefox): assets go by type or extension.
            entry("GET", "https://stats.io/collect?e=view", None, json!([]), None),
            entry("GET", "https://stats.io/logo.png", None, json!([]), None),
        ]);
        let r = import(&root, None).unwrap();
        let c = &r.collection;
        assert_eq!(c.vars["base"], api, "the most common origin");
        assert_eq!(c.name, "api.shop.io");
        assert_eq!(c.source.as_deref(), Some("HAR · WebInspector · 5 requests"));
        let urls: Vec<_> = c.endpoints.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            ["{{base}}/orders", "{{base}}/orders", "{{base}}/login", "{{base}}/upload", "https://stats.io/collect"]
        );
        assert_eq!(c.endpoints[0].group.as_deref(), Some("api.shop.io"), "grouped by host when there are several");
        assert_eq!(c.endpoints[4].group.as_deref(), Some("stats.io"));

        let get = &c.endpoints[0];
        assert_eq!(get.method, HttpMethod::Get);
        assert_eq!(
            get.query.iter().map(|q| (q.key.as_str(), q.value.as_str())).collect::<Vec<_>>(),
            [("page", "2"), ("sort", "new")]
        );
        assert!(matches!(&get.auth, Auth::Bearer { token } if token == "abc"));
        assert_eq!(header(get, "X-Tenant"), Some("acme"));
        assert!(
            header(get, ":authority").is_none()
                && header(get, "sec-fetch-mode").is_none()
                && header(get, "Authorization").is_none()
        );

        let post = &c.endpoints[1];
        assert!(matches!(&post.body, Body::Json { content } if content == "{\"sku\":1}"));
        assert!(post.headers.is_empty(), "content type and length come from the body");
        assert!(matches!(&c.endpoints[2].body, Body::Form { fields } if fields[1].value == "x y"));
        let Body::Multipart { fields } = &c.endpoints[3].body else { panic!("multipart") };
        assert_eq!((fields[0].kind, fields[1].kind), (FieldKind::Text, FieldKind::File));

        let warnings = r.warnings.join("\n");
        // app.js by its `_resourceType`; logo.png (none) by its extension.
        assert!(warnings.contains("fonts): 2;") && warnings.contains("repeated requests: 1"), "{warnings}");
        assert!(warnings.contains("CORS preflights: 1") && warnings.contains("ws:…): 1"), "{warnings}");
        assert!(warnings.contains("a.png") && warnings.contains("credentials"), "{warnings}");
    }

    #[test]
    fn says_why_nothing_was_imported() {
        let only_assets = har(vec![entry("GET", "https://a.io/x.css", Some("stylesheet"), json!([]), None)]);
        let err = import(&only_assets, None).unwrap_err();
        assert!(err.contains("no API requests") && err.contains("fonts): 1"), "{err}");
        assert!(import(&har(vec![]), None).unwrap_err().contains("no requests"));
    }

    #[test]
    fn one_host_is_not_grouped_and_the_name_can_be_given() {
        let root = har(vec![entry("DELETE", "http://localhost:8089/items/7", Some("fetch"), json!([]), None)]);
        let r = import(&root, Some("Mine")).unwrap();
        assert_eq!(r.collection.name, "Mine");
        assert_eq!(r.collection.endpoints[0].group, None);
        assert_eq!(r.collection.endpoints[0].url, "{{base}}/items/7");
        assert_eq!(r.collection.vars["base"], "http://localhost:8089");
    }
}
