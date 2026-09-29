//! Turns an [`Endpoint`] into a request that can be rendered many times cheaply.

use std::borrow::Cow;

use base64::Engine;
use bytes::Bytes;
use url::Url;

use super::{Bound, GenState, Scope, Template, TemplateError};
#[cfg(test)]
use crate::model::store::NoFiles;
use crate::model::{
    ApiKeyLocation, Auth, Body, Endpoint, FieldKind, HttpMethod, KeyValue, Secrets, Workspace, store::FileSource,
};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("unknown environment `{0}`")]
    UnknownEnvironment(String),
    #[error("undefined variable{}: {}{}", if .names.len() == 1 { "" } else { "s" }, .names.join(", "), match .environment {
        Some(env) => format!(" (define it in environment `{env}` or the collection)"),
        None => " (define it in the collection, or select an environment)".to_owned(),
    })]
    MissingVars { names: Vec<String>, environment: Option<String> },
    #[error("{0}")]
    BadUrl(String),
    #[error("choose a file for form field `{0}`")]
    NoFile(String),
    #[error("can't read uploaded file `{0}` (upload it again): {1}")]
    FileUnreadable(String, String),
}

pub struct CompiledRequest {
    pub method: HttpMethod,
    url: Bound,
    query: Vec<(String, Bound)>,
    headers: Vec<(String, Bound)>,
    body: Option<CompiledBody>,
    basic: Option<(Bound, Bound)>,
    /// Header carrying an API key, if the endpoint uses one; redacted in samples.
    pub api_key_header: Option<String>,
    /// Secret values in scope; redacted wherever they appear in samples.
    pub secret_values: Vec<String>,
    generators: GenState,
}

enum CompiledBody {
    /// Content type and template.
    Text(String, Bound),
    Form(Vec<(String, Bound)>),
    /// Boundary (fixed per compile) and parts.
    Multipart(String, Vec<Part>),
}

struct Part {
    name: String,
    value: PartValue,
}

enum PartValue {
    Text(Bound),
    /// Loaded once at compile; `Bytes` clones are cheap.
    File {
        filename: String,
        content_type: String,
        bytes: Bytes,
    },
}

/// Quotes and line breaks would end a Content-Disposition header early (HTML's escaping rules).
fn escape_disposition(s: &str) -> String {
    s.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A")
}

impl CompiledBody {
    fn uses_size(&self) -> bool {
        match self {
            Self::Text(_, b) => b.uses_size(),
            Self::Form(fields) => fields.iter().any(|(_, v)| v.uses_size()),
            Self::Multipart(_, parts) => parts.iter().any(|p| matches!(&p.value, PartValue::Text(v) if v.uses_size())),
        }
    }

    /// Content type, body bytes, and (when the bytes aren't all text) a readable version.
    fn render(&self, state: &GenState, n: u64) -> (String, Bytes, Option<String>) {
        match self {
            Self::Text(content_type, template) => (content_type.clone(), template.render_sized(state, n).into(), None),
            Self::Form(fields) => {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                for (k, v) in fields {
                    form.append_pair(k, &v.render_sized(state, n));
                }
                ("application/x-www-form-urlencoded".to_owned(), form.finish().into(), None)
            }
            Self::Multipart(boundary, parts) => {
                let mut body = Vec::new();
                // File contents are replaced by a placeholder in the readable version.
                let has_files = parts.iter().any(|p| matches!(p.value, PartValue::File { .. }));
                let mut display = has_files.then(String::new);
                let mut push = |bytes: &[u8], text: &str| {
                    body.extend_from_slice(bytes);
                    if let Some(d) = display.as_mut() {
                        d.push_str(text);
                    }
                };
                for part in parts {
                    let name = escape_disposition(&part.name);
                    match &part.value {
                        PartValue::Text(v) => {
                            let s = format!(
                                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{}\r\n",
                                v.render_sized(state, n)
                            );
                            push(s.as_bytes(), &s);
                        }
                        PartValue::File { filename, content_type, bytes } => {
                            let head = format!(
                                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
                                escape_disposition(filename)
                            );
                            push(head.as_bytes(), &head);
                            push(bytes, &format!("<file: {filename}, {} bytes>", bytes.len()));
                            push(b"\r\n", "\r\n");
                        }
                    }
                }
                let end = format!("--{boundary}--\r\n");
                push(end.as_bytes(), &end);
                (format!("multipart/form-data; boundary={boundary}"), body.into(), display)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RenderedRequest {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Bytes>,
    /// Readable body for display when `body` holds file contents.
    pub body_display: Option<String>,
}

impl RenderedRequest {
    /// The body as text for showing to people (file contents replaced by a placeholder).
    pub fn body_text(&self) -> Option<Cow<'_, str>> {
        match (&self.body_display, &self.body) {
            (Some(display), _) => Some(Cow::Borrowed(display)),
            (None, Some(body)) => Some(String::from_utf8_lossy(body)),
            (None, None) => None,
        }
    }
}

impl CompiledRequest {
    /// [`Self::compile_with`] for requests that don't upload files.
    #[cfg(test)]
    pub fn compile(
        endpoint: &Endpoint,
        workspace: &Workspace,
        secrets: &Secrets,
        environment: Option<&str>,
        mask_secrets: bool,
    ) -> Result<Self, CompileError> {
        Self::compile_with(endpoint, workspace, secrets, &NoFiles, environment, mask_secrets)
    }

    /// `environment` overrides the workspace's active one. With `mask_secrets`, secret values are
    /// replaced by [`super::MASK`] (for previews; never for sending). Uploaded files for multipart
    /// bodies are read from `files` now, once.
    pub fn compile_with(
        endpoint: &Endpoint,
        workspace: &Workspace,
        secrets: &Secrets,
        files: &dyn FileSource,
        environment: Option<&str>,
        mask_secrets: bool,
    ) -> Result<Self, CompileError> {
        let env_name = environment.map(str::to_owned).or_else(|| workspace.active_environment.clone());
        let env = match &env_name {
            Some(name) => {
                Some(workspace.environment(name).ok_or_else(|| CompileError::UnknownEnvironment(name.clone()))?)
            }
            None => None,
        };
        let scope = Scope {
            vars: env.map(|e| &e.vars),
            secrets: env_name.as_ref().and_then(|n| secrets.get(n)),
            collection_vars: workspace.collection_of(endpoint.id).map(|c| &c.vars),
            mask_secrets,
        };

        let mut missing = Vec::new();
        let mut bind =
            |src: &str| -> Result<Bound, CompileError> { Ok(Template::parse(src)?.bind(&scope, &mut missing)) };

        let url = bind(endpoint.url.trim())?;
        let mut query = bind_pairs(&endpoint.query, &mut bind)?;
        let mut headers = bind_pairs(&endpoint.headers, &mut bind)?;

        let body = match &endpoint.body {
            Body::None => None,
            Body::Json { content } => Some(CompiledBody::Text("application/json".to_owned(), bind(content)?)),
            Body::Raw { content_type, content } => Some(CompiledBody::Text(content_type.clone(), bind(content)?)),
            Body::Form { fields } => Some(CompiledBody::Form(bind_pairs(fields, &mut bind)?)),
            Body::Multipart { fields } => {
                let mut parts = Vec::new();
                for field in fields.iter().filter(|f| f.enabled && !f.key.trim().is_empty()) {
                    let name = field.key.trim().to_owned();
                    let value = match (field.kind, &field.file) {
                        (FieldKind::Text, _) => PartValue::Text(bind(&field.value)?),
                        (FieldKind::File, None) => return Err(CompileError::NoFile(name)),
                        (FieldKind::File, Some(file)) => PartValue::File {
                            filename: file.name.clone(),
                            content_type: file.content_type.clone(),
                            bytes: files
                                .read_file(file.id)
                                .map_err(|e| CompileError::FileUnreadable(file.name.clone(), format!("{e:#}")))?,
                        },
                    };
                    parts.push(Part { name, value });
                }
                Some(CompiledBody::Multipart(format!("----kestrel{}", uuid::Uuid::new_v4().simple()), parts))
            }
        };

        let mut basic = None;
        let mut api_key_header = None;
        match &endpoint.auth {
            Auth::None => {}
            Auth::Bearer { token } => headers.push(("Authorization".into(), bind(&format!("Bearer {token}"))?)),
            Auth::Basic { username, password } => basic = Some((bind(username)?, bind(password)?)),
            Auth::ApiKey { location: ApiKeyLocation::Header, name, value } => {
                headers.push((name.clone(), bind(value)?));
                api_key_header = Some(name.clone());
            }
            Auth::ApiKey { location: ApiKeyLocation::Query, name, value } => query.push((name.clone(), bind(value)?)),
        }

        if !missing.is_empty() {
            return Err(CompileError::MissingVars { names: missing, environment: env_name });
        }

        let compiled = Self {
            method: endpoint.method,
            url,
            query,
            headers,
            body,
            basic,
            api_key_header,
            secret_values: scope.secret_values(),
            generators: GenState::default(),
        };
        // Fail at start, not on every request, if the URL can never be valid.
        compiled.render_with(&GenState::default()).map_err(CompileError::BadUrl)?;
        Ok(compiled)
    }

    /// Renders without advancing the run's generators (`{{seq}}` etc.), for inspecting the target
    /// before a run starts.
    pub fn preview(&self) -> Result<RenderedRequest, String> {
        self.render_with(&GenState::default())
    }

    pub fn render(&self) -> Result<RenderedRequest, String> {
        self.render_with(&self.generators)
    }

    /// Renders with size n for `{{n…}}` generators (Big-O runs).
    pub fn render_sized(&self, n: u64) -> Result<RenderedRequest, String> {
        self.render_sized_with(&self.generators, n)
    }

    /// Whether the request has a size generator anywhere, i.e. can be used for a Big-O run.
    pub fn uses_size(&self) -> bool {
        self.url.uses_size()
            || self.query.iter().chain(&self.headers).any(|(_, v)| v.uses_size())
            || self.body.as_ref().is_some_and(CompiledBody::uses_size)
    }

    fn render_with(&self, state: &GenState) -> Result<RenderedRequest, String> {
        self.render_sized_with(state, 1)
    }

    fn render_sized_with(&self, state: &GenState, n: u64) -> Result<RenderedRequest, String> {
        let raw_url = self.url.render_sized(state, n);
        let mut url = Url::parse(&raw_url).map_err(|e| format!("invalid URL `{raw_url}`: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("unsupported URL scheme `{}` (use http or https)", url.scheme()));
        }
        if !self.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &self.query {
                pairs.append_pair(k, &v.render_sized(state, n));
            }
        }

        let mut headers: Vec<(String, String)> =
            self.headers.iter().map(|(k, v)| (k.clone(), v.render_sized(state, n))).collect();
        if let Some((user, pass)) = &self.basic {
            let creds = format!("{}:{}", user.render(state), pass.render(state));
            let encoded = base64::engine::general_purpose::STANDARD.encode(creds);
            headers.push(("Authorization".into(), format!("Basic {encoded}")));
        }

        let mut body_display = None;
        let body = self.body.as_ref().map(|compiled| {
            let (content_type, body, display) = compiled.render(state, n);
            body_display = display;
            // A hand-set multipart Content-Type can't know our boundary.
            if matches!(compiled, CompiledBody::Multipart(..)) {
                headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
            }
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                headers.push(("Content-Type".into(), content_type));
            }
            body
        });

        Ok(RenderedRequest { method: self.method, url, headers, body, body_display })
    }
}

fn bind_pairs(
    pairs: &[KeyValue],
    bind: &mut impl FnMut(&str) -> Result<Bound, CompileError>,
) -> Result<Vec<(String, Bound)>, CompileError> {
    pairs
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| Ok((kv.key.trim().to_owned(), bind(&kv.value)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::{Environment, FormField};

    fn endpoint(url: &str, auth: Auth) -> Endpoint {
        Endpoint {
            id: uuid::Uuid::new_v4(),
            name: String::new(),
            group: None,
            method: HttpMethod::Post,
            url: url.into(),
            headers: vec![KeyValue { key: "X-Req".into(), value: "{{uuid}}".into(), enabled: true }],
            query: vec![
                KeyValue { key: "page".into(), value: "{{seq}}".into(), enabled: true },
                KeyValue { key: "off".into(), value: "x".into(), enabled: false },
            ],
            body: Body::Json { content: r#"{"id":{{seq}}}"#.into() },
            auth,
            expect: None,
            extract: vec![],
        }
    }

    fn workspace() -> (Workspace, Secrets) {
        let ws = Workspace {
            environments: vec![Environment {
                name: "local".into(),
                vars: BTreeMap::from([("base".into(), "http://127.0.0.1:8080".into())]),
            }],
            active_environment: Some("local".into()),
            ..Default::default()
        };
        let secrets = Secrets::from([("local".into(), BTreeMap::from([("token".into(), "hunter2".into())]))]);
        (ws, secrets)
    }

    #[test]
    fn renders_url_query_headers_body_and_auth() {
        let (ws, secrets) = workspace();
        let ep = endpoint("{{base}}/echo", Auth::Bearer { token: "{{token}}".into() });
        let req = CompiledRequest::compile(&ep, &ws, &secrets, None, false).unwrap().render().unwrap();

        assert_eq!(req.url.as_str(), "http://127.0.0.1:8080/echo?page=1");
        assert_eq!(req.body_text().as_deref(), Some(r#"{"id":2}"#));
        let header = |name: &str| req.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(header("Authorization"), Some("Bearer hunter2"));
        assert_eq!(header("Content-Type"), Some("application/json"));
        assert_eq!(header("X-Req").map(str::len), Some(36));
    }

    #[test]
    fn form_and_multipart_bodies() {
        let (ws, secrets) = workspace();
        let fields = vec![
            KeyValue { key: "user name".into(), value: "a&b={{token}}".into(), enabled: true },
            KeyValue { key: "off".into(), value: "x".into(), enabled: false },
            KeyValue { key: "id".into(), value: "{{seq}}".into(), enabled: true },
        ];
        let header = |req: &RenderedRequest| {
            req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).unwrap().1.clone()
        };

        let mut ep = endpoint("{{base}}/", Auth::None);
        ep.body = Body::Form { fields: fields.clone() };
        let req = CompiledRequest::compile(&ep, &ws, &secrets, None, false).unwrap().render().unwrap();
        assert_eq!(header(&req), "application/x-www-form-urlencoded");
        assert_eq!(req.body_text().as_deref(), Some("user+name=a%26b%3Dhunter2&id=2"));

        ep.body = Body::Multipart {
            fields: fields.into_iter().map(|f| FormField::text(f.key, f.value, f.enabled)).collect(),
        };
        ep.headers.push(KeyValue { key: "content-type".into(), value: "multipart/form-data".into(), enabled: true });
        let req = CompiledRequest::compile(&ep, &ws, &secrets, None, false).unwrap().render().unwrap();
        let ct = header(&req);
        let boundary =
            ct.strip_prefix("multipart/form-data; boundary=").expect("our boundary replaces the hand-set header");
        let body = req.body_text().unwrap().into_owned();
        assert!(
            body.starts_with(&format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"user name\"\r\n\r\na&b=hunter2\r\n"
            )),
            "{body}"
        );
        assert!(body.contains("name=\"id\"\r\n\r\n2\r\n"), "{body}");
        assert!(!body.contains("name=\"off\""));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn multipart_file_parts() {
        use crate::model::{FileRef, store::WorkspaceStore};
        let (ws, secrets) = workspace();
        let store = WorkspaceStore::in_memory(Workspace::default(), Secrets::default());
        let png = Bytes::from_static(b"\x89PNG\r\n\x00\xff");
        let id = store.save_file(png.clone()).unwrap();
        let file = FileRef { id, name: "cat \"1\".png".into(), content_type: "image/png".into(), size: 8 };

        let mut ep = endpoint("{{base}}/", Auth::None);
        let mut photo = FormField::text("photo", "", true);
        photo.kind = FieldKind::File;
        ep.body = Body::Multipart { fields: vec![FormField::text("title", "cat", true), photo.clone()] };
        let err = CompiledRequest::compile_with(&ep, &ws, &secrets, &store, None, false).err().unwrap();
        assert_eq!(err.to_string(), "choose a file for form field `photo`");

        photo.file = Some(file);
        ep.body = Body::Multipart { fields: vec![FormField::text("title", "cat", true), photo] };
        let req = CompiledRequest::compile_with(&ep, &ws, &secrets, &store, None, false).unwrap().render().unwrap();
        let body = req.body.clone().unwrap();
        let head = b"Content-Disposition: form-data; name=\"photo\"; filename=\"cat %221%22.png\"\r\nContent-Type: image/png\r\n\r\n";
        let at = body.windows(head.len()).position(|w| w == head).expect("file part header");
        assert_eq!(&body[at + head.len()..at + head.len() + png.len()], &png[..], "raw bytes, not UTF-8 mangled");

        let text = req.body_text().unwrap();
        assert!(text.contains("<file: cat \"1\".png, 8 bytes>"), "{text}");
        assert!(text.contains("name=\"title\"\r\n\r\ncat\r\n"), "{text}");
    }

    #[test]
    fn basic_auth_and_masking() {
        let (ws, secrets) = workspace();
        let ep = endpoint("{{base}}/", Auth::Basic { username: "me".into(), password: "{{token}}".into() });
        let real = CompiledRequest::compile(&ep, &ws, &secrets, None, false).unwrap();
        assert_eq!(real.secret_values, vec!["hunter2".to_string()]);
        let auth = real.render().unwrap().headers.into_iter().find(|(k, _)| k == "Authorization").unwrap().1;
        assert_eq!(auth, "Basic bWU6aHVudGVyMg==");

        let ep = endpoint("{{base}}/?t={{token}}", Auth::None);
        let masked = CompiledRequest::compile(&ep, &ws, &secrets, None, true).unwrap().render().unwrap();
        assert!(!masked.url.as_str().contains("hunter2"));
    }

    #[test]
    fn environment_overrides_collection_defaults() {
        use crate::model::Collection;
        let (mut ws, secrets) = workspace();
        let ep = endpoint("{{base}}/{{path}}", Auth::None);
        ws.collections.push(Collection {
            id: uuid::Uuid::new_v4(),
            name: "api".into(),
            source: None,
            schema_defs: None,
            vars: BTreeMap::from([
                ("base".into(), "http://collection-default".into()),
                ("path".into(), "from-collection".into()),
            ]),
            endpoints: vec![ep.clone()],
        });
        let req = CompiledRequest::compile(&ep, &ws, &secrets, None, false).unwrap().render().unwrap();
        // `base` comes from the active environment, `path` only exists on the collection.
        assert_eq!(req.url.as_str(), "http://127.0.0.1:8080/from-collection?page=1");

        let req = CompiledRequest::compile(&ep, &Workspace { active_environment: None, ..ws }, &secrets, None, false)
            .unwrap()
            .render()
            .unwrap();
        assert_eq!(req.url.as_str(), "http://collection-default/from-collection?page=1");
    }

    #[test]
    fn explains_missing_vars_and_bad_urls() {
        let (ws, secrets) = workspace();
        let err =
            CompiledRequest::compile(&endpoint("{{host}}/x", Auth::None), &ws, &secrets, None, false).err().unwrap();
        assert_eq!(err.to_string(), "undefined variable: host (define it in environment `local` or the collection)");

        let err = CompiledRequest::compile(&endpoint("localhost:8080/x", Auth::None), &ws, &secrets, None, false)
            .err()
            .unwrap();
        assert!(matches!(err, CompileError::BadUrl(_)), "{err}");

        let err = CompiledRequest::compile(&endpoint("{{base}}", Auth::None), &ws, &secrets, Some("prod"), false)
            .err()
            .unwrap();
        assert!(matches!(err, CompileError::UnknownEnvironment(_)));
    }
}
