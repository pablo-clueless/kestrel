//! Turns an [`Endpoint`] into a request that can be rendered many times cheaply.

use base64::Engine;
use url::Url;

use super::{Bound, GenState, Scope, Template, TemplateError};
use crate::model::{ApiKeyLocation, Auth, Body, Endpoint, HttpMethod, KeyValue, Secrets, Workspace};

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
}

pub struct CompiledRequest {
    pub method: HttpMethod,
    url: Bound,
    query: Vec<(String, Bound)>,
    headers: Vec<(String, Bound)>,
    body: Option<(String, Bound)>,
    basic: Option<(Bound, Bound)>,
    /// Header carrying an API key, if the endpoint uses one; redacted in samples.
    pub api_key_header: Option<String>,
    /// Secret values in scope; redacted wherever they appear in samples.
    pub secret_values: Vec<String>,
    generators: GenState,
}

#[derive(Debug, Clone)]
pub struct RenderedRequest {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl CompiledRequest {
    /// `environment` overrides the workspace's active one. With `mask_secrets`, secret values are
    /// replaced by [`super::MASK`] (for previews; never for sending).
    pub fn compile(
        endpoint: &Endpoint,
        workspace: &Workspace,
        secrets: &Secrets,
        environment: Option<&str>,
        mask_secrets: bool,
    ) -> Result<Self, CompileError> {
        let env_name = environment.map(str::to_owned).or_else(|| workspace.active_environment.clone());
        let env = match &env_name {
            Some(name) => Some(workspace.environment(name).ok_or_else(|| CompileError::UnknownEnvironment(name.clone()))?),
            None => None,
        };
        let scope = Scope {
            vars: env.map(|e| &e.vars),
            secrets: env_name.as_ref().and_then(|n| secrets.get(n)),
            collection_vars: workspace.collection_of(endpoint.id).map(|c| &c.vars),
            mask_secrets,
        };

        let mut missing = Vec::new();
        let mut bind = |src: &str| -> Result<Bound, CompileError> { Ok(Template::parse(src)?.bind(&scope, &mut missing)) };

        let url = bind(endpoint.url.trim())?;
        let mut query = bind_pairs(&endpoint.query, &mut bind)?;
        let mut headers = bind_pairs(&endpoint.headers, &mut bind)?;

        let body = match &endpoint.body {
            Body::None => None,
            Body::Json { content } => Some(("application/json".to_owned(), bind(content)?)),
            Body::Raw { content_type, content } => Some((content_type.clone(), bind(content)?)),
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

    fn render_with(&self, state: &GenState) -> Result<RenderedRequest, String> {
        let raw_url = self.url.render(state);
        let mut url = Url::parse(&raw_url).map_err(|e| format!("invalid URL `{raw_url}`: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("unsupported URL scheme `{}` (use http or https)", url.scheme()));
        }
        if !self.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &self.query {
                pairs.append_pair(k, &v.render(state));
            }
        }

        let mut headers: Vec<(String, String)> =
            self.headers.iter().map(|(k, v)| (k.clone(), v.render(state))).collect();
        if let Some((user, pass)) = &self.basic {
            let creds = format!("{}:{}", user.render(state), pass.render(state));
            let encoded = base64::engine::general_purpose::STANDARD.encode(creds);
            headers.push(("Authorization".into(), format!("Basic {encoded}")));
        }

        let body = self.body.as_ref().map(|(content_type, template)| {
            if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                headers.push(("Content-Type".into(), content_type.clone()));
            }
            template.render(state)
        });

        Ok(RenderedRequest { method: self.method, url, headers, body })
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
    use crate::model::Environment;

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
        assert_eq!(req.body.as_deref(), Some(r#"{"id":2}"#));
        let header = |name: &str| req.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(header("Authorization"), Some("Bearer hunter2"));
        assert_eq!(header("Content-Type"), Some("application/json"));
        assert_eq!(header("X-Req").map(str::len), Some(36));
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
        let err = CompiledRequest::compile(&endpoint("{{host}}/x", Auth::None), &ws, &secrets, None, false)
            .err()
            .unwrap();
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
