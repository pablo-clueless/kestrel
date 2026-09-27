//! The workspace: collections of endpoints, and environments. Everything here round-trips through
//! `kestrel.json` and is exported to TypeScript. String fields marked "template" may contain `{{…}}`.

pub mod store;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workspace {
    #[serde(default)]
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(default)]
    pub active_environment: Option<String>,
    /// The collection shown expanded in the sidebar.
    #[serde(default)]
    pub active_collection: Option<Uuid>,
}

impl Workspace {
    pub fn endpoint(&self, id: Uuid) -> Option<&Endpoint> {
        self.collections.iter().flat_map(|c| &c.endpoints).find(|e| e.id == id)
    }

    /// The collection an endpoint belongs to.
    pub fn collection_of(&self, endpoint_id: Uuid) -> Option<&Collection> {
        self.collections.iter().find(|c| c.endpoints.iter().any(|e| e.id == endpoint_id))
    }

    pub fn environment(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }
}

/// A named set of endpoints, usually one API (one imported spec).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    /// Defaults for `{{…}}` in this collection's endpoints, e.g. `base`. The active environment's
    /// variables and secrets override these.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub endpoints: Vec<Endpoint>,
    /// Where the collection came from, e.g. "OpenAPI 3.0.3 · Petstore 1.0.0". None if made by hand.
    #[serde(default)]
    pub source: Option<String>,
    /// Shared schema definitions from the spec (`{"components": {"schemas": …}}` or `{"definitions": …}`),
    /// stored once so response schemas can keep their `$ref`s (recursive schemas included).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub schema_defs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Endpoint {
    pub id: Uuid,
    #[serde(default)]
    pub name: String,
    /// Tag or folder; used to group the endpoint list.
    #[serde(default)]
    pub group: Option<String>,
    pub method: HttpMethod,
    /// Template, e.g. `{{base}}/users/{{id}}`.
    pub url: String,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub query: Vec<KeyValue>,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub auth: Auth,
    /// Responses the spec declares, for contract checks. None for hand-made endpoints.
    #[serde(default)]
    pub expect: Option<Expectation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Expectation {
    pub responses: Vec<ExpectedResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExpectedResponse {
    /// `"200"`, a range like `"2XX"`, or `"default"`.
    pub status: String,
    /// JSON Schema (2020-12) for a JSON body. `$ref`s point into the collection's `schema_defs`.
    #[serde(default)]
    #[ts(type = "unknown")]
    pub schema: Option<serde_json::Value>,
}

/// Own enum rather than `http::Method`, which doesn't derive `TS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "UPPERCASE")]
#[ts(export)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl From<HttpMethod> for reqwest::Method {
    fn from(m: HttpMethod) -> Self {
        match m {
            HttpMethod::Get => Self::GET,
            HttpMethod::Post => Self::POST,
            HttpMethod::Put => Self::PUT,
            HttpMethod::Patch => Self::PATCH,
            HttpMethod::Delete => Self::DELETE,
            HttpMethod::Head => Self::HEAD,
            HttpMethod::Options => Self::OPTIONS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyValue {
    pub key: String,
    /// Template.
    pub value: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Body {
    #[default]
    None,
    /// Template. Sent as `application/json`.
    Json { content: String },
    /// Template. Sent with the given content type.
    Raw { content_type: String, content: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Auth {
    #[default]
    None,
    /// `token` is a template, normally `{{token}}` pointing at a secret.
    Bearer { token: String },
    Basic { username: String, password: String },
    ApiKey { location: ApiKeyLocation, name: String, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ApiKeyLocation {
    Header,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Environment {
    pub name: String,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

/// Secret values per environment. Lives in `kestrel.secrets.json`; never sent to the UI.
pub type Secrets = BTreeMap<String, BTreeMap<String, String>>;

/// `GET /api/workspace`. Secrets are write-only, so only their names are returned.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceResponse {
    pub workspace: Workspace,
    /// Environment name → names of the secrets set for it.
    pub secret_keys: BTreeMap<String, Vec<String>>,
}

/// `POST /api/render` and `POST /api/send`. Takes the endpoint inline so unsaved drafts can be tried.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TryRequest {
    pub endpoint: Endpoint,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u32>,
}

/// `PUT /api/secrets`. `value: null` deletes the secret.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetSecretRequest {
    pub environment: String,
    pub key: String,
    pub value: Option<String>,
}
