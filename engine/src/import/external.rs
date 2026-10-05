//! External `$ref`s (`schemas.yaml#/Pet`, `https://…/common.json#/Error`) for OpenAPI / Swagger
//! specs. HANDOFF → Inputs.
//!
//! The importer only follows local refs (`#/…`), so external ones are bundled first: each referenced
//! document is fetched once and mounted inside the spec at `#/x-kestrel-external/<i>`, and every
//! ref (in the spec and in the fetched documents) is rewritten to point there. References stay
//! references, so recursive schemas survive, and contract checks resolve them the same way as
//! `#/components/schemas/…`. Relative refs need the spec's own URL, so they resolve only for specs
//! imported by URL; absolute http(s) refs resolve either way.

use std::{collections::HashMap, future::Future};

use serde_json::Value;
use url::Url;

/// Where fetched documents are mounted in the spec.
pub const MOUNT: &str = "x-kestrel-external";
/// Most documents fetched for one import.
const MAX_DOCS: usize = 20;
/// Most bytes fetched for one import, all documents together.
const MAX_BYTES: usize = 10 * 1024 * 1024;

/// Whether `root` has any `$ref` that isn't local.
pub fn has_external_refs(root: &Value) -> bool {
    match root {
        Value::Object(map) => {
            map.get("$ref").and_then(Value::as_str).is_some_and(|r| !r.starts_with('#'))
                || map.values().any(has_external_refs)
        }
        Value::Array(items) => items.iter().any(has_external_refs),
        _ => false,
    }
}

/// Bundles `root`'s external refs, fetching documents with `fetch`. `base` is where the spec came
/// from (for relative refs). Returns the warnings: refs that couldn't be resolved stay as they were,
/// and the importer treats them as it always has (unresolved).
pub async fn bundle<F, Fut>(root: &mut Value, base: Option<&Url>, fetch: F) -> Vec<String>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let mut b = Bundler::default();
    b.rewrite(root, base, None);
    let mut docs: Vec<Value> = Vec::new();
    let mut fetched = 0usize;
    let mut i = 0;
    while i < b.urls.len() {
        let url = b.urls[i].clone();
        let mut doc = match fetch(url.clone()).await {
            Ok(text) if fetched + text.len() > MAX_BYTES => {
                b.warn(format!(
                    "Stopped fetching referenced documents at {} MB; `{url}` wasn't loaded.",
                    MAX_BYTES >> 20
                ));
                Value::Null
            }
            Ok(text) => {
                fetched += text.len();
                match super::parse(&text) {
                    Ok(doc) => doc,
                    Err(e) => {
                        b.warn(format!("The referenced document `{url}` couldn't be read: {e}."));
                        Value::Null
                    }
                }
            }
            Err(e) => {
                b.warn(format!("The referenced document `{url}` couldn't be fetched: {e}."));
                Value::Null
            }
        };
        b.rewrite(&mut doc, Some(&url), Some(i));
        docs.push(doc);
        i += 1;
    }
    if !docs.is_empty()
        && let Value::Object(map) = root
    {
        map.insert(MOUNT.into(), Value::Array(docs));
    }
    b.warnings
}

#[derive(Default)]
struct Bundler {
    /// Documents to fetch, in mount order.
    urls: Vec<Url>,
    index: HashMap<Url, usize>,
    warnings: Vec<String>,
}

impl Bundler {
    fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }

    /// Rewrites every ref in `v`, which lives in the spec (`current` None) or in mounted document
    /// `current`, whose own URL is `base`.
    fn rewrite(&mut self, v: &mut Value, base: Option<&Url>, current: Option<usize>) {
        match v {
            Value::Object(map) => {
                if let Some(Value::String(r)) = map.get_mut("$ref")
                    && let Some(new) = self.target(r, base, current)
                {
                    *r = new;
                }
                for (key, child) in map.iter_mut() {
                    if key != "$ref" {
                        self.rewrite(child, base, current);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.rewrite(item, base, current)),
            _ => {}
        }
    }

    /// The local ref that `r` becomes, or None to leave it alone.
    fn target(&mut self, r: &str, base: Option<&Url>, current: Option<usize>) -> Option<String> {
        let (path, fragment) = r.split_once('#').unwrap_or((r, ""));
        let pointer = |doc: usize| format!("#/{MOUNT}/{doc}{fragment}");
        if path.is_empty() {
            // Local to its own document: only refs inside a mounted document move.
            return current.map(pointer);
        }
        let url = match (Url::parse(path), base) {
            (Ok(url), _) => url,
            (Err(_), Some(base)) => match base.join(path) {
                Ok(url) => url,
                Err(e) => {
                    self.warn(format!("The $ref `{r}` isn't a valid address ({e}); it wasn't followed."));
                    return None;
                }
            },
            (Err(_), None) => {
                self.warn(format!(
                    "The $ref `{r}` points at another file, which can't be found from pasted text or an uploaded \
                     file. Import the spec by URL so relative files can be fetched."
                ));
                return None;
            }
        };
        if !matches!(url.scheme(), "http" | "https") {
            self.warn(format!("The $ref `{r}` isn't an http(s) address; it wasn't followed."));
            return None;
        }
        let mut doc_url = url;
        doc_url.set_fragment(None);
        let doc = match self.index.get(&doc_url) {
            Some(&doc) => doc,
            None if self.urls.len() >= MAX_DOCS => {
                self.warn(format!("Stopped at {MAX_DOCS} referenced documents; `{doc_url}` wasn't loaded."));
                return None;
            }
            None => {
                let doc = self.urls.len();
                self.index.insert(doc_url.clone(), doc);
                self.urls.push(doc_url);
                doc
            }
        };
        Some(pointer(doc))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    async fn run(root: &mut Value, base: Option<&str>, files: &[(&str, &str)]) -> Vec<String> {
        let files: HashMap<String, String> = files.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        let base = base.map(|b| Url::parse(b).unwrap());
        bundle(root, base.as_ref(), |url: Url| {
            let found = files.get(url.as_str()).cloned();
            async move { found.ok_or_else(|| "404".to_owned()) }
        })
        .await
    }

    #[tokio::test]
    async fn mounts_documents_and_rewrites_refs_everywhere() {
        let mut root = json!({
            "openapi": "3.0.3",
            "paths": { "/pets": { "get": { "responses": { "200": { "content": { "application/json": {
                "schema": { "$ref": "schemas/pet.yaml#/Pet" } } } } } } } },
            "components": { "schemas": { "Local": { "$ref": "#/components/schemas/Other" } } }
        });
        let pet = "Pet:\n  type: object\n  properties:\n    tag: { $ref: '#/Tag' }\n    error: { $ref: '../common.json#/Error' }\n    self: { $ref: '#/Pet' }\nTag: { type: string }\n";
        let common = r#"{ "Error": { "type": "object" } }"#;
        let warnings = run(
            &mut root,
            Some("https://api.example.com/v1/openapi.yaml"),
            &[("https://api.example.com/v1/schemas/pet.yaml", pet), ("https://api.example.com/v1/common.json", common)],
        )
        .await;
        assert!(warnings.is_empty(), "{warnings:?}");
        let schema = &root.pointer("/paths/~1pets/get/responses/200/content/application~1json/schema").unwrap();
        assert_eq!(schema["$ref"], "#/x-kestrel-external/0/Pet");
        assert_eq!(root["components"]["schemas"]["Local"]["$ref"], "#/components/schemas/Other", "local refs stay");
        let pet = &root[MOUNT][0]["Pet"]["properties"];
        assert_eq!(pet["tag"]["$ref"], "#/x-kestrel-external/0/Tag", "a document's own refs follow it");
        assert_eq!(pet["error"]["$ref"], "#/x-kestrel-external/1/Error", "relative to the document");
        assert_eq!(pet["self"]["$ref"], "#/x-kestrel-external/0/Pet", "recursion stays a reference");
        assert_eq!(root[MOUNT][1]["Error"]["type"], "object");
    }

    #[tokio::test]
    async fn says_what_it_could_not_follow() {
        let mut pasted = json!({ "a": { "$ref": "pet.yaml#/Pet" }, "b": { "$ref": "https://x.io/missing.json" } });
        let warnings = run(&mut pasted, None, &[]).await;
        assert!(warnings[0].contains("Import the spec by URL"), "{warnings:?}");
        assert!(warnings[1].contains("couldn't be fetched"), "{warnings:?}");
        assert_eq!(pasted["a"]["$ref"], "pet.yaml#/Pet", "an unresolvable ref is left as it was");
        assert!(has_external_refs(&json!({ "x": [{ "$ref": "a.yaml" }] })));
        assert!(!has_external_refs(&json!({ "x": { "$ref": "#/a" } })));
    }
}
