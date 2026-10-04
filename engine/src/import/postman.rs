//! Postman collection (v2.0 / v2.1 JSON) → collection. HANDOFF → Inputs.
//!
//! Postman's `{{name}}` variables are the same syntax as ours, so URLs, headers and bodies are kept
//! as written. Folders become groups ("Folder / Subfolder"); collection variables become this
//! collection's variables (they're defaults that an environment overrides, in both tools); `:id`
//! path variables become `{{id}}`. Auth is inherited from folders and the collection as Postman does.
//! Scripts (pre-request, tests) aren't run, and that's reported.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use uuid::Uuid;

use super::ImportResult;
use crate::model::{ApiKeyLocation, Auth, Body, Collection, Endpoint, FieldKind, FormField, HttpMethod, KeyValue};

/// Whether `root` is a Postman collection.
pub fn is_collection(root: &Value) -> bool {
    let info = root.get("info");
    info.and_then(|i| i.get("_postman_id")).is_some()
        || info
            .and_then(|i| i.get("schema"))
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains("schema.getpostman.com") && s.contains("collection"))
}

/// Whether `root` is a Postman environment export, which isn't a collection.
pub fn is_environment(root: &Value) -> bool {
    root.get("_postman_variable_scope").and_then(Value::as_str) == Some("environment")
        || (root.get("values").is_some_and(Value::is_array) && root.get("item").is_none() && root.get("info").is_none())
}

pub fn import(root: &Value, name: Option<&str>) -> Result<ImportResult, String> {
    let info = root.get("info").unwrap_or(&Value::Null);
    let title = info.get("name").and_then(Value::as_str).unwrap_or("Postman collection").to_owned();
    let version = match info.get("schema").and_then(Value::as_str) {
        Some(s) if s.contains("v2.0") => "Postman v2.0",
        Some(s) if s.contains("v1") => return Err("Postman v1 collections aren't supported; export it as v2.1".into()),
        _ => "Postman v2.1",
    };

    let mut ctx = Ctx { warnings: Vec::new(), vars: BTreeMap::new(), endpoints: Vec::new(), scripts: false };
    for var in root.get("variable").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(key), Some(value)) = (var.get("key").and_then(Value::as_str), var.get("value"))
            && var.get("disabled").and_then(Value::as_bool) != Some(true)
        {
            ctx.vars.insert(key.to_owned(), scalar(value));
        }
    }
    ctx.note_scripts(root);
    let auth = root.get("auth");
    ctx.items(root.get("item"), &[], auth);
    if ctx.endpoints.is_empty() {
        ctx.warnings.push("The collection has no requests.".into());
    }
    if ctx.scripts {
        ctx.warnings.push(
            "The collection has pre-request or test scripts. They aren't run; anything they set (tokens, \
             variables) has to be set by hand."
                .into(),
        );
    }

    let collection = Collection {
        headers: Vec::new(),
        id: Uuid::new_v4(),
        name: name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&title).to_owned(),
        vars: ctx.vars,
        endpoints: ctx.endpoints,
        groups: Vec::new(),
        source: Some(format!("{version} · {title}")),
        schema_defs: None,
    };
    let mut warnings = ctx.warnings;
    warnings.dedup();
    Ok(ImportResult { collection, format: version.into(), warnings })
}

struct Ctx {
    warnings: Vec<String>,
    vars: BTreeMap<String, String>,
    endpoints: Vec<Endpoint>,
    scripts: bool,
}

impl Ctx {
    /// Walks `item` (requests and folders). `folders` is the path so far; `auth` the inherited auth.
    fn items(&mut self, items: Option<&Value>, folders: &[String], auth: Option<&Value>) {
        for item in items.and_then(Value::as_array).into_iter().flatten() {
            self.note_scripts(item);
            let name = item.get("name").and_then(Value::as_str).unwrap_or("").trim().to_owned();
            // A folder's own auth applies to what's inside it, unless it says to inherit.
            let own_auth = item.get("auth").filter(|a| auth_type(a) != Some("inherit"));
            if item.get("item").is_some() {
                let mut path = folders.to_vec();
                path.push(if name.is_empty() { "Folder".into() } else { name });
                self.items(item.get("item"), &path, own_auth.or(auth));
            } else if let Some(request) = item.get("request") {
                let group = (!folders.is_empty()).then(|| folders.join(" / "));
                let endpoint = self.endpoint(&name, group, request, auth);
                self.endpoints.push(endpoint);
            }
        }
    }

    fn note_scripts(&mut self, node: &Value) {
        let has_script = node.get("event").and_then(Value::as_array).into_iter().flatten().any(|e| {
            e.get("script").and_then(|s| s.get("exec")).is_some_and(|exec| match exec {
                Value::Array(lines) => lines.iter().any(|l| l.as_str().is_some_and(|l| !l.trim().is_empty())),
                Value::String(s) => !s.trim().is_empty(),
                _ => false,
            })
        });
        self.scripts |= has_script;
    }

    fn endpoint(&mut self, name: &str, group: Option<String>, request: &Value, inherited: Option<&Value>) -> Endpoint {
        // A request can be just a URL string.
        let request = match request {
            Value::String(url) => &Value::Object(Map::from_iter([("url".to_owned(), Value::String(url.clone()))])),
            other => other,
        };
        let label = if name.is_empty() { "a request" } else { name };
        let method = self.method(request.get("method").and_then(Value::as_str), label);
        let (url, mut query) = self.url(request.get("url"));
        let mut headers = key_values(request.get("header"));

        let auth = match request.get("auth").filter(|a| auth_type(a) != Some("inherit")) {
            Some(own) => self.auth(own, label),
            None => inherited.map_or(Auth::None, |a| self.auth(a, label)),
        };
        if let Auth::ApiKey { location: ApiKeyLocation::Query, .. } = &auth {
            // Already part of the auth; don't send it twice if the export also listed it.
            query.retain(|q| !matches!(&auth, Auth::ApiKey { name, .. } if &q.key == name));
        }

        let body = self.body(request.get("body"), &mut headers, label);
        let body = self.dynamic_body(body);
        Endpoint {
            id: Uuid::new_v4(),
            name: if name.is_empty() { url.clone() } else { name.to_owned() },
            group,
            method,
            url: dynamic(&url, &mut self.warnings),
            headers: headers.into_iter().map(|kv| self.dynamic_kv(kv)).collect(),
            query: query.into_iter().map(|kv| self.dynamic_kv(kv)).collect(),
            body,
            auth,
            expect: None,
            extract: Vec::new(),
        }
    }

    fn dynamic_kv(&mut self, kv: KeyValue) -> KeyValue {
        KeyValue { value: dynamic(&kv.value, &mut self.warnings), ..kv }
    }

    fn dynamic_body(&mut self, body: Body) -> Body {
        match body {
            Body::Json { content } => Body::Json { content: dynamic(&content, &mut self.warnings) },
            Body::Raw { content_type, content } => {
                Body::Raw { content_type, content: dynamic(&content, &mut self.warnings) }
            }
            Body::Form { fields } => Body::Form { fields: fields.into_iter().map(|kv| self.dynamic_kv(kv)).collect() },
            Body::Multipart { fields } => Body::Multipart {
                fields: fields
                    .into_iter()
                    .map(|f| FormField { value: dynamic(&f.value, &mut self.warnings), ..f })
                    .collect(),
            },
            Body::None => Body::None,
        }
    }

    fn method(&mut self, raw: Option<&str>, label: &str) -> HttpMethod {
        match raw.unwrap_or("GET").to_ascii_uppercase().as_str() {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "PATCH" => HttpMethod::Patch,
            "DELETE" => HttpMethod::Delete,
            "HEAD" => HttpMethod::Head,
            "OPTIONS" => HttpMethod::Options,
            other => {
                self.warnings.push(format!("{label}: the method {other} isn't supported; it was imported as GET."));
                HttpMethod::Get
            }
        }
    }

    /// The URL without its query, and the query as rows (disabled ones kept, switched off). Path
    /// variables (`/:id`) become `{{id}}`, with their values as collection variables.
    fn url(&mut self, url: Option<&Value>) -> (String, Vec<KeyValue>) {
        let Some(url) = url else { return (String::new(), Vec::new()) };
        let raw = match url {
            Value::String(s) => s.clone(),
            other => other.get("raw").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| build_raw(other)),
        };
        let (base, raw_query) = match raw.split_once('?') {
            Some((b, q)) => (b.to_owned(), Some(q.to_owned())),
            None => (raw.clone(), None),
        };
        let query = match url.get("query").and_then(Value::as_array) {
            Some(_) => key_values(url.get("query")),
            None => raw_query
                .map(|q| {
                    q.split('&')
                        .filter(|p| !p.is_empty())
                        .map(|p| {
                            let (k, v) = p.split_once('=').unwrap_or((p, ""));
                            KeyValue { key: k.to_owned(), value: v.to_owned(), enabled: true }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };

        // `:name` path segments → `{{name}}`.
        for var in url.get("variable").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(key), Some(value)) = (var.get("key").and_then(Value::as_str), var.get("value")) {
                let value = scalar(value);
                if !value.is_empty() {
                    self.vars.entry(key.to_owned()).or_insert(value);
                }
            }
        }
        let base = base
            .split('/')
            .map(|segment| match segment.strip_prefix(':') {
                Some(name) if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') => {
                    format!("{{{{{name}}}}}")
                }
                _ => segment.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("/");
        (base, query)
    }

    /// The body, by Postman's `mode`. A `Content-Type` header the body type already implies is dropped.
    fn body(&mut self, body: Option<&Value>, headers: &mut Vec<KeyValue>, label: &str) -> Body {
        let Some(body) = body.filter(|b| b.get("disabled").and_then(Value::as_bool) != Some(true)) else {
            return Body::None;
        };
        let content_type = headers
            .iter()
            .rev()
            .find(|h| h.enabled && h.key.eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.clone());
        let drop_content_type =
            |headers: &mut Vec<KeyValue>| headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));

        match body.get("mode").and_then(Value::as_str) {
            Some("raw") => {
                let content = body.get("raw").and_then(Value::as_str).unwrap_or("").to_owned();
                if content.is_empty() {
                    return Body::None;
                }
                let language = body
                    .get("options")
                    .and_then(|o| o.get("raw"))
                    .and_then(|r| r.get("language"))
                    .and_then(Value::as_str);
                let json_header = content_type.as_deref().is_some_and(|ct| ct.trim().starts_with("application/json"));
                if language == Some("json") && content_type.as_deref().is_none_or(|_| json_header) || json_header {
                    drop_content_type(headers);
                    return Body::Json { content };
                }
                let content_type = content_type.unwrap_or_else(|| {
                    match language {
                        Some("xml") => "application/xml",
                        Some("html") => "text/html",
                        Some("javascript") => "application/javascript",
                        _ => "text/plain",
                    }
                    .to_owned()
                });
                drop_content_type(headers);
                Body::Raw { content_type, content }
            }
            Some("urlencoded") => {
                drop_content_type(headers);
                Body::Form { fields: key_values(body.get("urlencoded")) }
            }
            Some("formdata") => {
                drop_content_type(headers);
                let fields = body
                    .get("formdata")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|f| {
                        let key = f.get("key").and_then(Value::as_str)?.to_owned();
                        let enabled = f.get("disabled").and_then(Value::as_bool) != Some(true);
                        let is_file = f.get("type").and_then(Value::as_str) == Some("file");
                        if is_file {
                            self.warnings
                                .push(format!("{label}: choose the file for the form field `{key}` in the Body tab."));
                        }
                        Some(FormField {
                            key,
                            kind: if is_file { FieldKind::File } else { FieldKind::Text },
                            value: if is_file { String::new() } else { f.get("value").map(scalar).unwrap_or_default() },
                            file: None,
                            enabled,
                        })
                    })
                    .collect();
                Body::Multipart { fields }
            }
            Some("graphql") => {
                let gql = body.get("graphql").unwrap_or(&Value::Null);
                let query = gql.get("query").and_then(Value::as_str).unwrap_or("");
                let variables = match gql.get("variables") {
                    Some(Value::String(s)) if !s.trim().is_empty() => {
                        serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
                    }
                    Some(v @ Value::Object(_)) => v.clone(),
                    _ => Value::Object(Map::new()),
                };
                drop_content_type(headers);
                let content =
                    serde_json::to_string_pretty(&serde_json::json!({ "query": query, "variables": variables }))
                        .unwrap_or_default();
                Body::Json { content }
            }
            Some("file") => {
                self.warnings
                    .push(format!("{label}: the body is a file, which can't be imported; add it in the Body tab."));
                Body::None
            }
            _ => Body::None,
        }
    }

    /// Bearer, Basic or API key. Other schemes are reported and left off.
    fn auth(&mut self, auth: &Value, label: &str) -> Auth {
        let Some(kind) = auth_type(auth) else { return Auth::None };
        let param = |name: &str| auth_param(auth, kind, name);
        let result = match kind {
            "noauth" => Auth::None,
            "bearer" => Auth::Bearer { token: param("token").unwrap_or_default() },
            "basic" => Auth::Basic {
                username: param("username").unwrap_or_default(),
                password: param("password").unwrap_or_default(),
            },
            "apikey" => Auth::ApiKey {
                location: if param("in").as_deref() == Some("query") {
                    ApiKeyLocation::Query
                } else {
                    ApiKeyLocation::Header
                },
                name: param("key").unwrap_or_else(|| "X-API-Key".into()),
                value: param("value").unwrap_or_default(),
            },
            other => {
                self.warnings.push(format!("{label}: {other} auth isn't supported; set the auth by hand."));
                return Auth::None;
            }
        };
        let literal = |s: &str| !s.is_empty() && !s.trim().starts_with("{{");
        let has_literal_secret = match &result {
            Auth::Bearer { token } => literal(token),
            Auth::Basic { password, .. } => literal(password),
            Auth::ApiKey { value, .. } => literal(value),
            Auth::None => false,
        };
        if has_literal_secret {
            self.warnings.push(
                "Some requests carry credentials as plain text, which are saved in the collection. Consider \
                 moving them into an environment secret and using {{name}} instead."
                    .into(),
            );
        }
        result
    }
}

fn auth_type(auth: &Value) -> Option<&str> {
    auth.get("type").and_then(Value::as_str)
}

/// One auth parameter. v2.1 lists them as `[{key, value}]`; v2.0 as an object.
fn auth_param(auth: &Value, kind: &str, name: &str) -> Option<String> {
    match auth.get(kind)? {
        Value::Array(params) => params
            .iter()
            .find(|p| p.get("key").and_then(Value::as_str) == Some(name))
            .and_then(|p| p.get("value"))
            .map(scalar),
        Value::Object(map) => map.get(name).map(scalar),
        _ => None,
    }
}

/// `[{key, value, disabled}]` → rows. Disabled ones are kept, switched off, as Postman shows them.
fn key_values(list: Option<&Value>) -> Vec<KeyValue> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|kv| {
            let key = kv.get("key").and_then(Value::as_str)?.to_owned();
            Some(KeyValue {
                key,
                value: kv.get("value").map(scalar).unwrap_or_default(),
                enabled: kv.get("disabled").and_then(Value::as_bool) != Some(true),
            })
        })
        .collect()
}

/// A URL from its parts, for exports without `raw`.
fn build_raw(url: &Value) -> String {
    let join = |key: &str, sep: &str| match url.get(key) {
        Some(Value::Array(parts)) => parts.iter().map(scalar).collect::<Vec<_>>().join(sep),
        Some(other) => scalar(other),
        None => String::new(),
    };
    let protocol = url.get("protocol").and_then(Value::as_str);
    let host = join("host", ".");
    let port = url.get("port").map(scalar).filter(|p| !p.is_empty());
    let path = join("path", "/");
    let mut out = String::new();
    if let Some(p) = protocol {
        out.push_str(p);
        out.push_str("://");
    }
    out.push_str(&host);
    if let Some(port) = port {
        out.push(':');
        out.push_str(&port);
    }
    if !path.is_empty() {
        out.push('/');
        out.push_str(&path);
    }
    out
}

/// Strings as they are; numbers and booleans as text.
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Postman's dynamic variables (`{{$guid}}`) → our generators where there's one. Others are kept
/// and reported, since they'd otherwise be read as an ordinary (missing) variable.
fn dynamic(text: &str, warnings: &mut Vec<String>) -> String {
    if !text.contains("{{$") {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{$") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match name {
            "guid" | "randomUUID" => out.push_str("{{uuid}}"),
            "randomInt" => out.push_str("{{int:0..1000}}"),
            other => {
                warnings.push(format!(
                    "Postman's {{{{${other}}}}} has no equivalent here; replace it with a value or a generator \
                     ({{{{uuid}}}}, {{{{seq}}}}, {{{{int:1..100}}}})."
                ));
                out.push_str(&rest[start..start + 3 + end + 2]);
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn collection() -> Value {
        json!({
            "info": {
                "_postman_id": "1",
                "name": "Shop API",
                "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
            },
            "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{token}}", "type": "string" }] },
            "variable": [{ "key": "baseUrl", "value": "https://shop.example.com" }, { "key": "off", "value": "x", "disabled": true }],
            "item": [
                {
                    "name": "Orders",
                    "item": [
                        {
                            "name": "Get order",
                            "request": {
                                "method": "GET",
                                "header": [{ "key": "Accept", "value": "application/json" }, { "key": "X-Debug", "value": "1", "disabled": true }],
                                "url": {
                                    "raw": "{{baseUrl}}/orders/:orderId?expand=items&trace=1",
                                    "host": ["{{baseUrl}}"],
                                    "path": ["orders", ":orderId"],
                                    "query": [{ "key": "expand", "value": "items" }, { "key": "trace", "value": "1", "disabled": true }],
                                    "variable": [{ "key": "orderId", "value": "42" }]
                                }
                            }
                        },
                        {
                            "name": "Admin",
                            "auth": { "type": "basic", "basic": [{ "key": "username", "value": "ada" }, { "key": "password", "value": "hunter22" }] },
                            "item": [{
                                "name": "Create order",
                                "event": [{ "listen": "prerequest", "script": { "exec": ["pm.variables.set('x', 1)"] } }],
                                "request": {
                                    "method": "POST",
                                    "header": [{ "key": "Content-Type", "value": "application/json" }],
                                    "body": { "mode": "raw", "raw": "{ \"id\": \"{{$guid}}\", \"n\": {{$randomInt}} }", "options": { "raw": { "language": "json" } } },
                                    "url": "{{baseUrl}}/orders"
                                }
                            }]
                        }
                    ]
                },
                {
                    "name": "Upload",
                    "request": {
                        "method": "POST",
                        "auth": { "type": "noauth" },
                        "body": { "mode": "formdata", "formdata": [{ "key": "note", "value": "hi", "type": "text" }, { "key": "file", "type": "file", "src": "/tmp/a.png" }] },
                        "url": { "raw": "{{baseUrl}}/upload" }
                    }
                },
                {
                    "name": "Login form",
                    "request": {
                        "method": "POST",
                        "auth": { "type": "apikey", "apikey": [{ "key": "key", "value": "api_key" }, { "key": "value", "value": "{{apiKey}}" }, { "key": "in", "value": "query" }] },
                        "body": { "mode": "urlencoded", "urlencoded": [{ "key": "user", "value": "ada" }] },
                        "url": "{{baseUrl}}/login"
                    }
                },
                {
                    "name": "Search",
                    "request": {
                        "method": "POST",
                        "body": { "mode": "graphql", "graphql": { "query": "{ items { id } }", "variables": "{\"first\": 5}" } },
                        "url": "{{baseUrl}}/graphql"
                    }
                }
            ]
        })
    }

    fn find<'a>(c: &'a Collection, name: &str) -> &'a Endpoint {
        c.endpoints.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no `{name}`"))
    }

    #[test]
    fn imports_folders_variables_urls_and_auth() {
        let root = collection();
        assert!(is_collection(&root) && !is_environment(&root));
        let r = import(&root, None).unwrap();
        let c = &r.collection;
        assert_eq!((r.format.as_str(), c.name.as_str()), ("Postman v2.1", "Shop API"));
        assert_eq!(c.source.as_deref(), Some("Postman v2.1 · Shop API"));
        assert_eq!(c.vars.get("baseUrl").map(String::as_str), Some("https://shop.example.com"));
        assert!(!c.vars.contains_key("off"), "disabled variables are skipped");
        assert_eq!(c.vars.get("orderId").map(String::as_str), Some("42"), "path variable value");

        let get = find(c, "Get order");
        assert_eq!(get.group.as_deref(), Some("Orders"));
        assert_eq!(get.url, "{{baseUrl}}/orders/{{orderId}}");
        let q: Vec<_> = get.query.iter().map(|q| (q.key.as_str(), q.enabled)).collect();
        assert_eq!(q, [("expand", true), ("trace", false)], "disabled rows kept, switched off");
        assert!(get.headers.iter().any(|h| h.key == "X-Debug" && !h.enabled));
        assert!(matches!(&get.auth, Auth::Bearer { token } if token == "{{token}}"), "inherited from the collection");

        let create = find(c, "Create order");
        assert_eq!(create.group.as_deref(), Some("Orders / Admin"), "nested folders");
        assert!(matches!(&create.auth, Auth::Basic { username, .. } if username == "ada"), "the folder's auth wins");
        match &create.body {
            Body::Json { content } => assert_eq!(content, r#"{ "id": "{{uuid}}", "n": {{int:0..1000}} }"#),
            other => panic!("{other:?}"),
        }
        assert!(create.headers.iter().all(|h| !h.key.eq_ignore_ascii_case("content-type")));

        let upload = find(c, "Upload");
        assert!(matches!(upload.auth, Auth::None), "noauth overrides the collection's");
        assert!(matches!(&upload.body, Body::Multipart { fields } if fields[1].kind == FieldKind::File));

        let login = find(c, "Login form");
        assert!(matches!(&login.auth, Auth::ApiKey { location: ApiKeyLocation::Query, name, .. } if name == "api_key"));
        assert!(matches!(&login.body, Body::Form { fields } if fields[0].key == "user"));

        match &find(c, "Search").body {
            Body::Json { content } => {
                let v: Value = serde_json::from_str(content).unwrap();
                assert_eq!(v, json!({ "query": "{ items { id } }", "variables": { "first": 5 } }));
            }
            other => panic!("{other:?}"),
        }

        for expected in ["scripts", "`file`", "plain text"] {
            assert!(r.warnings.iter().any(|w| w.contains(expected)), "{expected}: {:?}", r.warnings);
        }
    }

    #[test]
    fn v20_auth_objects_and_unknown_dynamic_values() {
        let root = json!({
            "info": { "name": "Old", "schema": "https://schema.getpostman.com/json/collection/v2.0.0/collection.json" },
            "item": [{
                "name": "x",
                "request": {
                    "url": "https://x.io/a?t={{$timestamp}}",
                    "auth": { "type": "bearer", "bearer": { "token": "abc" } }
                }
            }]
        });
        let r = import(&root, Some("Mine")).unwrap();
        let e = &r.collection.endpoints[0];
        assert_eq!((r.format.as_str(), r.collection.name.as_str()), ("Postman v2.0", "Mine"));
        assert!(matches!(&e.auth, Auth::Bearer { token } if token == "abc"));
        assert_eq!(e.query[0].value, "{{$timestamp}}", "kept as written");
        assert!(r.warnings.iter().any(|w| w.contains("$timestamp")), "{:?}", r.warnings);
    }

    #[test]
    fn environments_are_told_apart() {
        let env = json!({ "name": "Prod", "values": [{ "key": "baseUrl", "value": "https://x" }], "_postman_variable_scope": "environment" });
        assert!(is_environment(&env) && !is_collection(&env));
    }
}
