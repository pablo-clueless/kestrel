//! Scrubs credentials from anything we show or store: sampled requests/responses in reports and
//! `/api/send` results. Services like httpbin echo request headers back in the body, so body text is
//! scrubbed too, not just headers.

use crate::template::request::RenderedRequest;

pub const REDACTED: &str = "[redacted]";

const SENSITIVE_HEADERS: [&str; 4] = ["authorization", "cookie", "set-cookie", "proxy-authorization"];
/// Shorter values would redact half of every body.
const MIN_SECRET_LEN: usize = 3;

pub struct Redactor {
    headers: Vec<String>,
    /// Longest first, so a secret that contains another is replaced whole.
    values: Vec<String>,
}

impl Redactor {
    /// `sent` contributes the values of its sensitive headers (e.g. a Basic auth blob, which isn't
    /// itself a secret value but encodes one).
    pub fn new(api_key_header: Option<&str>, secret_values: &[String], sent: Option<&RenderedRequest>) -> Self {
        let mut headers: Vec<String> = SENSITIVE_HEADERS.iter().map(|h| (*h).to_owned()).collect();
        if let Some(h) = api_key_header {
            headers.push(h.to_ascii_lowercase());
        }

        let mut values: Vec<String> = secret_values.to_vec();
        if let Some(req) = sent {
            for (name, value) in &req.headers {
                if headers.contains(&name.to_ascii_lowercase()) {
                    values.push(value.clone());
                    // Also the bare credential after a scheme like "Bearer ".
                    if let Some((_, cred)) = value.split_once(' ') {
                        values.push(cred.to_owned());
                    }
                }
            }
        }
        values.retain(|v| v.len() >= MIN_SECRET_LEN);
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        values.dedup();
        Self { headers, values }
    }

    pub fn header(&self, name: &str, value: &str) -> String {
        if self.headers.contains(&name.to_ascii_lowercase()) { REDACTED.to_owned() } else { self.text(value) }
    }

    pub fn headers<'a>(&self, pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), self.header(k, v))).collect()
    }

    pub fn text(&self, text: &str) -> String {
        self.values.iter().fold(text.to_owned(), |acc, secret| acc.replace(secret.as_str(), REDACTED))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;

    #[test]
    fn scrubs_headers_and_echoed_bodies() {
        let sent = RenderedRequest {
            method: HttpMethod::Get,
            url: "http://x/".parse().unwrap(),
            headers: vec![("Authorization".into(), "Basic bWU6aHVudGVyMg==".into())],
            body: None,
            body_display: None,
        };
        let r = Redactor::new(Some("X-Api-Key"), &["hunter2".into(), "k".into()], Some(&sent));

        assert_eq!(r.header("authorization", "Basic abc"), REDACTED);
        assert_eq!(r.header("x-api-key", "whatever"), REDACTED);
        assert_eq!(r.header("Accept", "a/b"), "a/b");

        let echoed = r#"{"headers":{"Authorization":"Basic bWU6aHVudGVyMg=="},"q":"hunter2","k":"k"}"#;
        let scrubbed = r.text(echoed);
        assert!(!scrubbed.contains("bWU6"));
        assert!(!scrubbed.contains("hunter2"));
        assert!(scrubbed.contains(r#""k":"k""#), "short values are left alone: {scrubbed}");
    }
}
