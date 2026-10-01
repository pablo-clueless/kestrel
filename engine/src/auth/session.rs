//! Session tokens and the cookie that carries them. A token is 256 random bits; the database keeps
//! only its SHA-256, so a database leak doesn't hand out live sessions. Sessions, not JWTs, so
//! logging out takes effect at once.

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, header};
use sha2::{Digest, Sha256};

pub const LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Over plain http (local use, `fly proxy`).
const COOKIE: &str = "kestrel_session";
/// Over https: `__Host-` makes browsers require `Secure`, `Path=/` and no `Domain`.
const SECURE_COOKIE: &str = "__Host-kestrel_session";

/// A new token, hex encoded for the cookie.
pub fn new_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The session token from the request's cookies, if any.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE || *name == SECURE_COOKIE)
        .map(|(_, value)| value.to_owned())
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `Set-Cookie` for a new session. `secure` when the request came in over https.
pub fn set_cookie(token: &str, secure: bool) -> HeaderValue {
    let max_age = LIFETIME.as_secs();
    let value = if secure {
        format!("{SECURE_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age={max_age}")
    } else {
        format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}")
    };
    HeaderValue::from_str(&value).expect("token is hex")
}

/// `Set-Cookie` that removes the session cookie.
pub fn clear_cookie(secure: bool) -> HeaderValue {
    HeaderValue::from_static(if secure {
        "__Host-kestrel_session=; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=0"
    } else {
        "kestrel_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_well_formed_tokens_from_either_cookie_name() {
        let token = new_token();
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("theme=dark; kestrel_session={token}").parse().unwrap());
        assert_eq!(from_headers(&headers), Some(token.clone()));

        headers.insert(header::COOKIE, format!("__Host-kestrel_session={token}").parse().unwrap());
        assert_eq!(from_headers(&headers), Some(token));

        headers.insert(header::COOKIE, "kestrel_session=not-a-token".parse().unwrap());
        assert_eq!(from_headers(&headers), None);
        assert_ne!(new_token(), new_token());
        assert_eq!(hash("x").len(), 32);
    }
}
