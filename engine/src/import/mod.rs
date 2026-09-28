//! Spec → collection. OpenAPI 3.0 / 3.1 and Swagger 2.0, in JSON or YAML.
//! HANDOFF → Inputs.
//!
//! Works on `serde_json::Value` rather than typed models so one code path covers all three
//! versions, and so response schemas can be kept verbatim (with their `$ref`s) for contract checks.
//! Only local `$ref`s (`#/…`) are supported.

mod schema;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;
use uuid::Uuid;

use crate::model::{
    ApiKeyLocation, Auth, Body, Collection, Endpoint, Expectation, ExpectedResponse, HttpMethod, KeyValue,
};
use schema::{example_string, resolve, synthesize, to_json_schema};

const METHODS: [(&str, HttpMethod); 7] = [
    ("get", HttpMethod::Get),
    ("post", HttpMethod::Post),
    ("put", HttpMethod::Put),
    ("patch", HttpMethod::Patch),
    ("delete", HttpMethod::Delete),
    ("head", HttpMethod::Head),
    ("options", HttpMethod::Options),
];

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum ImportSource {
    /// Pasted text or an uploaded file's contents.
    Text { content: String },
    /// Fetched by the engine.
    Url { url: String },
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportRequest {
    pub source: ImportSource,
    /// Overrides the spec's title as the collection name.
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /api/import`. The UI adds `collection` to the workspace.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportResult {
    pub collection: Collection,
    /// E.g. "OpenAPI 3.0.3".
    pub format: String,
    /// Things that were skipped or guessed, for the user to review.
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Version {
    Swagger2,
    OpenApi30,
    OpenApi31,
}

pub fn import(text: &str, name: Option<&str>) -> Result<ImportResult, String> {
    let root = parse(text)?;
    let (version, format) = detect(&root)?;
    let mut ctx = Ctx { root: &root, version, warnings: Vec::new(), vars: BTreeMap::new() };

    let base = ctx.base_url();
    ctx.vars.insert("base".into(), base);

    let mut endpoints = Vec::new();
    if let Some(paths) = root.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            let item = resolve(&root, item);
            for (key, method) in METHODS {
                if let Some(op) = item.get(key) {
                    endpoints.push(ctx.endpoint(path, item, op, method));
                }
            }
        }
    }
    if endpoints.is_empty() {
        ctx.warnings.push("The spec has no operations under `paths`.".into());
    }

    let info = root.get("info");
    let title = info.and_then(|i| i.get("title")).and_then(Value::as_str).unwrap_or("Imported API");
    let spec_version = info.and_then(|i| i.get("version")).and_then(Value::as_str);
    let collection = Collection {
        id: Uuid::new_v4(),
        name: name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(title).to_owned(),
        vars: ctx.vars.clone(),
        endpoints,
        source: Some(match spec_version {
            Some(v) => format!("{format} · {title} {v}"),
            None => format!("{format} · {title}"),
        }),
        schema_defs: ctx.schema_defs(),
    };
    let mut warnings = ctx.warnings;
    warnings.dedup();
    Ok(ImportResult { collection, format, warnings })
}

fn parse(text: &str) -> Result<Value, String> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return Err("the document is empty".into());
    }
    if trimmed.starts_with('{') {
        serde_json::from_str(trimmed).map_err(|e| format!("not valid JSON: {e}"))
    } else {
        serde_yaml_ng::from_str(trimmed).map_err(|e| format!("not valid JSON or YAML: {e}"))
    }
}

fn detect(root: &Value) -> Result<(Version, String), String> {
    if let Some(v) = root.get("openapi").and_then(Value::as_str) {
        return match v {
            _ if v.starts_with("3.0") => Ok((Version::OpenApi30, format!("OpenAPI {v}"))),
            _ if v.starts_with("3.1") => Ok((Version::OpenApi31, format!("OpenAPI {v}"))),
            _ => Err(format!("OpenAPI {v} isn't supported (3.0 and 3.1 are)")),
        };
    }
    if root.get("swagger").and_then(Value::as_str) == Some("2.0") {
        return Ok((Version::Swagger2, "Swagger 2.0".into()));
    }
    if root.get("info").and_then(|i| i.get("_postman_id")).is_some() {
        return Err("this looks like a Postman collection; Postman import isn't supported yet".into());
    }
    Err("not an OpenAPI 3.x or Swagger 2.0 document (no `openapi` or `swagger` field)".into())
}

struct Ctx<'a> {
    root: &'a Value,
    version: Version,
    warnings: Vec<String>,
    /// Collection variables: `base` plus one per path parameter.
    vars: BTreeMap<String, String>,
}

impl Ctx<'_> {
    fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }

    fn base_url(&mut self) -> String {
        if self.version == Version::Swagger2 {
            let Some(host) = self.root.get("host").and_then(Value::as_str) else {
                self.warn("No `host` in the spec; set `base` in the collection settings.");
                return String::new();
            };
            let scheme = self
                .root
                .get("schemes")
                .and_then(Value::as_array)
                .and_then(|s| {
                    s.iter().filter_map(Value::as_str).find(|s| *s == "https").or(s.first().and_then(Value::as_str))
                })
                .unwrap_or("https");
            let base_path = self.root.get("basePath").and_then(Value::as_str).unwrap_or("");
            return format!("{scheme}://{host}{}", base_path.trim_end_matches('/'));
        }

        let Some(server) = self.root.get("servers").and_then(Value::as_array).and_then(|s| s.first()) else {
            self.warn("No `servers` in the spec; set `base` in the collection settings.");
            return String::new();
        };
        let mut url = server.get("url").and_then(Value::as_str).unwrap_or("").to_owned();
        // Server variables: substitute their defaults.
        if let Some(vars) = server.get("variables").and_then(Value::as_object) {
            for (name, var) in vars {
                if let Some(default) = var.get("default").and_then(Value::as_str) {
                    url = url.replace(&format!("{{{name}}}"), default);
                }
            }
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.warn(format!(
                "The server URL `{url}` is relative; set `base` to the full URL in the collection settings."
            ));
        }
        url.trim_end_matches('/').to_owned()
    }

    fn schema_defs(&self) -> Option<Value> {
        let (key, defs) = match self.version {
            Version::Swagger2 => ("definitions", self.root.get("definitions")?),
            _ => ("components", self.root.get("components").and_then(|c| c.get("schemas"))?),
        };
        let defs = to_json_schema(defs, self.version == Version::OpenApi31);
        Some(match key {
            "definitions" => serde_json::json!({ "definitions": defs }),
            _ => serde_json::json!({ "components": { "schemas": defs } }),
        })
    }

    fn endpoint(&mut self, path: &str, item: &Value, op: &Value, method: HttpMethod) -> Endpoint {
        let op = resolve(self.root, op);
        let text = |k: &str| op.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let name = text("summary")
            .or_else(|| text("operationId"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} {path}", format!("{method:?}").to_uppercase()));
        let group =
            op.get("tags").and_then(Value::as_array).and_then(|t| t.first()).and_then(Value::as_str).map(str::to_owned);

        // `{petId}` → `{{petId}}`; the value lives in a collection variable.
        let url = format!("{{{{base}}}}{}", path.replace('{', "{{").replace('}', "}}"));

        let mut query = Vec::new();
        let mut headers = Vec::new();
        let mut body = Body::None;
        for param in self.parameters(item, op) {
            let Some(name) = param.get("name").and_then(Value::as_str).map(str::to_owned) else { continue };
            let location = param.get("in").and_then(Value::as_str).unwrap_or("");
            let required = param.get("required").and_then(Value::as_bool).unwrap_or(location == "path");
            match location {
                "path" => {
                    let value = self.param_example(&param);
                    self.vars.entry(name).or_insert(value);
                }
                "query" => query.push(KeyValue { key: name, value: self.param_example(&param), enabled: required }),
                "header" => {
                    let skip =
                        ["authorization", "content-type", "accept"].contains(&name.to_ascii_lowercase().as_str());
                    if !skip {
                        headers.push(KeyValue { key: name, value: self.param_example(&param), enabled: required });
                    }
                }
                "cookie" => self.warn("Cookie parameters aren't imported; add a Cookie header by hand."),
                // Swagger 2.0 bodies are parameters.
                "body" => {
                    let schema = param.get("schema").cloned().unwrap_or(Value::Null);
                    body = self.json_body(None, &schema);
                }
                "formData" => self.warn("Swagger formData parameters aren't imported; set the body by hand."),
                _ => {}
            }
        }
        if let Some(request_body) = op.get("requestBody") {
            body = self.request_body(resolve(self.root, request_body));
        }

        Endpoint {
            id: Uuid::new_v4(),
            name,
            group,
            method,
            url,
            headers,
            query,
            body,
            auth: self.auth(op),
            expect: self.expectation(op),
        }
    }

    /// Path-item parameters overridden by operation parameters with the same name and location.
    fn parameters(&self, item: &Value, op: &Value) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for list in [item.get("parameters"), op.get("parameters")].into_iter().flatten() {
            for p in list.as_array().into_iter().flatten() {
                let p = resolve(self.root, p).clone();
                let key = |v: &Value| (v.get("name").cloned(), v.get("in").cloned());
                out.retain(|existing| key(existing) != key(&p));
                out.push(p);
            }
        }
        out
    }

    fn param_example(&self, param: &Value) -> String {
        if let Some(v) = param.get("example") {
            return example_string(v);
        }
        if let Some(v) = param
            .get("examples")
            .and_then(Value::as_object)
            .and_then(|m| m.values().next())
            .map(|e| resolve(self.root, e))
            .and_then(|e| e.get("value"))
        {
            return example_string(v);
        }
        // 3.x keeps the type under `schema`; 2.0 puts it on the parameter itself.
        let schema = param.get("schema").unwrap_or(param);
        example_string(&synthesize(self.root, schema, false))
    }

    fn request_body(&mut self, rb: &Value) -> Body {
        let Some(content) = rb.get("content").and_then(Value::as_object) else { return Body::None };
        if let Some((_, media)) = content.iter().find(|(ct, _)| is_json(ct)) {
            let example = media_example(self.root, media);
            let schema = media.get("schema").cloned().unwrap_or(Value::Null);
            return self.json_body(example, &schema);
        }
        if let Some(media) = content.get("application/x-www-form-urlencoded") {
            let example = media_example(self.root, media)
                .unwrap_or_else(|| synthesize(self.root, media.get("schema").unwrap_or(&Value::Null), true));
            let form = example
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(&example_string(v))))
                        .collect::<Vec<_>>()
                        .join("&")
                })
                .unwrap_or_default();
            return Body::Raw { content_type: "application/x-www-form-urlencoded".into(), content: form };
        }
        let Some((content_type, media)) = content.iter().next() else { return Body::None };
        if content_type.starts_with("multipart/") {
            self.warn("Multipart request bodies aren't supported yet; those endpoints were imported without a body.");
            return Body::None;
        }
        let content = media_example(self.root, media).map(|v| example_string(&v)).unwrap_or_default();
        Body::Raw { content_type: content_type.clone(), content }
    }

    fn json_body(&self, example: Option<Value>, schema: &Value) -> Body {
        // Synthesized bodies use generators (`{{uuid}}`) so each request gets fresh values.
        let value = example.unwrap_or_else(|| synthesize(self.root, schema, true));
        Body::Json { content: serde_json::to_string_pretty(&value).unwrap_or_default() }
    }

    fn auth(&mut self, op: &Value) -> Auth {
        // Operation-level `security` overrides the root; `[]` means none.
        let security = op.get("security").or_else(|| self.root.get("security"));
        let Some(scheme_name) = security
            .and_then(Value::as_array)
            .and_then(|reqs| reqs.iter().find_map(|r| r.as_object().and_then(|o| o.keys().next().cloned())))
        else {
            return Auth::None;
        };
        let schemes = match self.version {
            Version::Swagger2 => self.root.get("securityDefinitions"),
            _ => self.root.get("components").and_then(|c| c.get("securitySchemes")),
        };
        let Some(scheme) = schemes.and_then(|s| s.get(&scheme_name)).map(|s| resolve(self.root, s)) else {
            self.warn(format!("Security scheme `{scheme_name}` isn't defined; imported without auth."));
            return Auth::None;
        };
        let kind = scheme.get("type").and_then(Value::as_str).unwrap_or("");
        let http_scheme = scheme.get("scheme").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        match kind {
            "http" if http_scheme == "bearer" => Auth::Bearer { token: "{{token}}".into() },
            "http" if http_scheme == "basic" => {
                Auth::Basic { username: "{{username}}".into(), password: "{{password}}".into() }
            }
            "basic" => Auth::Basic { username: "{{username}}".into(), password: "{{password}}".into() },
            "apiKey" => {
                let name = scheme.get("name").and_then(Value::as_str).unwrap_or("X-API-Key").to_owned();
                let location = match scheme.get("in").and_then(Value::as_str) {
                    Some("query") => ApiKeyLocation::Query,
                    Some("cookie") => {
                        self.warn("Cookie API keys aren't supported; imported as a header.");
                        ApiKeyLocation::Header
                    }
                    _ => ApiKeyLocation::Header,
                };
                Auth::ApiKey { location, name, value: "{{apiKey}}".into() }
            }
            "oauth2" | "openIdConnect" => {
                self.warn(
                    "OAuth2/OpenID Connect is imported as a Bearer token: put an access token in the `token` secret.",
                );
                Auth::Bearer { token: "{{token}}".into() }
            }
            other => {
                self.warn(format!("Security scheme type `{other}` isn't supported; imported without auth."));
                Auth::None
            }
        }
    }

    fn expectation(&self, op: &Value) -> Option<Expectation> {
        let responses = op.get("responses")?.as_object()?;
        let is_31 = self.version == Version::OpenApi31;
        let mut out = Vec::new();
        for (status, response) in responses {
            let response = resolve(self.root, response);
            let schema = match self.version {
                Version::Swagger2 => response.get("schema"),
                _ => response
                    .get("content")
                    .and_then(Value::as_object)
                    .and_then(|c| c.iter().find(|(ct, _)| is_json(ct)).map(|(_, m)| m))
                    .and_then(|m| m.get("schema")),
            };
            out.push(ExpectedResponse { status: status.clone(), schema: schema.map(|s| to_json_schema(s, is_31)) });
        }
        (!out.is_empty()).then_some(Expectation { responses: out })
    }
}

fn is_json(content_type: &str) -> bool {
    let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    ct == "application/json" || ct.ends_with("+json") || ct == "*/*"
}

fn media_example(root: &Value, media: &Value) -> Option<Value> {
    if let Some(v) = media.get("example") {
        return Some(v.clone());
    }
    let first: Option<&Map<String, Value>> = media.get("examples").and_then(Value::as_object);
    first.and_then(|m| m.values().next()).map(|e| resolve(root, e)).and_then(|e| e.get("value")).cloned()
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests;
