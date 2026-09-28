//! Pulls values out of a response for an endpoint's "After response" rules.

use serde_json::Value;

use crate::model::{Extract, ExtractSource};

/// The value `rule` selects, as text. Strings come back bare; other JSON values as compact JSON.
pub fn pick(rule: &Extract, status: u16, headers: &[(String, String)], body: &[u8]) -> Result<String, String> {
    match rule.source {
        ExtractSource::Status => Ok(status.to_string()),
        ExtractSource::Header => {
            let name = rule.path.trim();
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| format!("no `{name}` header in the response"))
        }
        ExtractSource::Body => {
            let path = rule.path.trim();
            if path.is_empty() {
                return Ok(String::from_utf8_lossy(body).into_owned());
            }
            let json: Value = serde_json::from_slice(body).map_err(|_| "the response body isn't JSON".to_string())?;
            match lookup(&json, path)? {
                Value::String(s) => Ok(s.clone()),
                Value::Null => Err(format!("`{path}` is null")),
                other => Ok(other.to_string()),
            }
        }
    }
}

enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

/// `data.items[0].id`, with an optional leading `$` / `$.`.
fn parse(path: &str) -> Result<Vec<Step<'_>>, String> {
    let path = path.strip_prefix('$').unwrap_or(path);
    let path = path.strip_prefix('.').unwrap_or(path);
    let mut steps = Vec::new();
    for segment in path.split('.') {
        let (key, mut rest) = segment.split_once('[').map_or((segment, ""), |(k, r)| (k, r));
        if !key.is_empty() {
            steps.push(Step::Key(key));
        } else if rest.is_empty() {
            return Err(format!("empty segment in `{path}`"));
        }
        while !rest.is_empty() {
            let (index, after) = rest.split_once(']').ok_or_else(|| format!("unclosed `[` in `{path}`"))?;
            steps.push(Step::Index(index.trim().parse().map_err(|_| format!("`[{index}]` isn't an index"))?));
            rest = after.strip_prefix('[').unwrap_or(after);
            if !after.is_empty() && !after.starts_with('[') {
                return Err(format!("unexpected `{after}` in `{path}`"));
            }
        }
    }
    Ok(steps)
}

fn lookup<'v>(json: &'v Value, path: &str) -> Result<&'v Value, String> {
    parse(path)?.iter().try_fold(json, |node, step| match step {
        Step::Key(k) => node.get(k).ok_or_else(|| format!("`{path}` not found (no `{k}`)")),
        Step::Index(i) => node.get(i).ok_or_else(|| format!("`{path}` not found (no [{i}])")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExtractTarget;

    fn rule(source: ExtractSource, path: &str) -> Extract {
        Extract { source, path: path.into(), target: ExtractTarget::Variable, name: "x".into(), enabled: true }
    }

    const BODY: &[u8] = br#"{"data":{"token":"abc","user":{"id":7}},"items":[{"id":"a"},{"id":"b"}],"n":null}"#;

    #[test]
    fn body_paths() {
        let body = |p| pick(&rule(ExtractSource::Body, p), 200, &[], BODY);
        assert_eq!(body("data.token").unwrap(), "abc");
        assert_eq!(body("$.data.user.id").unwrap(), "7");
        assert_eq!(body("items[1].id").unwrap(), "b");
        assert_eq!(body("data.user").unwrap(), r#"{"id":7}"#);
        assert!(body("data.missing").unwrap_err().contains("not found"));
        assert!(body("items[9]").is_err());
        assert!(body("n").unwrap_err().contains("null"));
        assert!(body("items[x]").is_err());
        assert_eq!(body("").unwrap(), String::from_utf8_lossy(BODY));
    }

    #[test]
    fn not_json() {
        assert!(pick(&rule(ExtractSource::Body, "a"), 200, &[], b"<html>").unwrap_err().contains("isn't JSON"));
    }

    #[test]
    fn headers_and_status() {
        let headers = vec![("X-Request-Id".to_string(), "r1".to_string())];
        assert_eq!(pick(&rule(ExtractSource::Header, "x-request-id"), 200, &headers, b"").unwrap(), "r1");
        assert!(pick(&rule(ExtractSource::Header, "etag"), 200, &headers, b"").is_err());
        assert_eq!(pick(&rule(ExtractSource::Status, ""), 201, &[], b"").unwrap(), "201");
    }
}
