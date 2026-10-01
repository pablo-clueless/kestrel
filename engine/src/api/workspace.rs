use std::{borrow::Cow, collections::HashSet, time::Duration};

use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::Deserialize;

use super::{AppState, Scope};
use crate::{
    engine::{
        client::{self, ClientOptions, Outcome},
        sample,
        types::SentRequest,
    },
    error::ApiError,
    extract,
    model::{
        ExtractTarget, FileRef, Saved, SendResponse, SetSecretRequest, TryRequest, Workspace, WorkspaceResponse,
        store::WorkspaceStore,
    },
    redact::Redactor,
    template::request::CompiledRequest,
};

const SEND_BODY_BYTES: usize = 256 * 1024;
const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn get(scope: Scope) -> Json<WorkspaceResponse> {
    Json(WorkspaceResponse { workspace: scope.store.workspace(), secret_keys: scope.store.secret_keys() })
}

pub async fn put(scope: Scope, Json(workspace): Json<Workspace>) -> Result<StatusCode, ApiError> {
    let mut names = HashSet::new();
    if let Some(dup) = workspace.environments.iter().find(|e| !names.insert(e.name.as_str())) {
        return Err(ApiError::BadRequest(format!("duplicate environment name `{}`", dup.name)));
    }
    let mut collection_ids = HashSet::new();
    if let Some(dup) = workspace.collections.iter().find(|c| !collection_ids.insert(c.id)) {
        return Err(ApiError::BadRequest(format!("duplicate collection id {}", dup.id)));
    }
    // Endpoint ids are unique across collections: runs and /send find endpoints by id alone.
    let mut ids = HashSet::new();
    if let Some(dup) = workspace.collections.iter().flat_map(|c| &c.endpoints).find(|e| !ids.insert(e.id)) {
        return Err(ApiError::BadRequest(format!("duplicate endpoint id {}", dup.id)));
    }
    scope.store.save_workspace(workspace).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_secret(scope: Scope, Json(req): Json<SetSecretRequest>) -> Result<StatusCode, ApiError> {
    if req.environment.trim().is_empty() || req.key.trim().is_empty() {
        return Err(ApiError::BadRequest("environment and key are required".into()));
    }
    scope
        .store
        .set_secret(req.environment.trim(), req.key.trim(), req.value)
        .await
        .map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UploadQuery {
    name: String,
}

/// `POST /api/files?name=…`: the raw file as the body, its type as `Content-Type`. Stored for
/// multipart file fields to reference.
pub async fn upload_file(
    scope: Scope,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Json<FileRef>, ApiError> {
    let name = q.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::BadRequest("a file name is required".into()));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let size = bytes.len() as u64;
    let id = scope.store.save_file(bytes).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    Ok(Json(FileRef { id, name, content_type, size }))
}

/// The request as it would be sent, with secrets masked.
pub async fn render(scope: Scope, Json(req): Json<TryRequest>) -> Result<Json<SentRequest>, ApiError> {
    let compiled = compile(&scope.store, &req, true).await?;
    let r = compiled.render().map_err(ApiError::BadRequest)?;
    let body = r.body_text().map(Cow::into_owned);
    Ok(Json(SentRequest { method: r.method, url: r.url.to_string(), headers: r.headers, body }))
}

/// The "try it" button: one request, full (redacted) response. Then the endpoint's extract rules run
/// against the unredacted response: secrets are stored here, variables are returned for the UI to save.
pub async fn send(
    State(state): State<AppState>,
    scope: Scope,
    Json(req): Json<TryRequest>,
) -> Result<Json<SendResponse>, ApiError> {
    let store = &scope.store;
    let compiled = compile(store, &req, false).await?;
    let rendered = compiled.render().map_err(ApiError::BadRequest)?;
    let opts = ClientOptions { keep_alive: true, follow_redirects: true, confirmed_hosts: store.confirmed_hosts() };
    let target = client::connect(&rendered.url, &opts).await.map_err(ApiError::BadRequest)?;
    let timeout = req.timeout_ms.map_or(DEFAULT_SEND_TIMEOUT, |ms| Duration::from_millis(ms.into()));
    let outcome = client::execute(&target.client, &rendered, timeout.min(state.config.caps.max_timeout)).await;

    let (saved, new_secrets) = run_extracts(store, &req, &outcome).await;
    // Hide secrets saved just now too, e.g. the token in a login response.
    let secret_values: Vec<String> = compiled.secret_values.iter().cloned().chain(new_secrets).collect();
    let redactor = Redactor::new(compiled.api_key_header.as_deref(), &secret_values, Some(&rendered));
    let mut sample = sample::build(&rendered, &outcome, &redactor, SEND_BODY_BYTES);
    // The draft endpoint carries its own `expect`; schema definitions come from its saved collection.
    let contract = crate::contract::for_endpoint(&store.workspace(), &req.endpoint).map_err(ApiError::BadRequest)?;
    if let (Some(contract), Some(status)) = (contract, outcome.status) {
        let check = contract.check(status, &outcome.body);
        sample.contract = Some(crate::contract::ContractCheck { message: redactor.text(&check.message), ..check });
    }
    Ok(Json(SendResponse { sample, saved }))
}

/// Applies the enabled extract rules. Returns each rule's outcome and the secret values stored.
async fn run_extracts(store: &WorkspaceStore, req: &TryRequest, outcome: &Outcome) -> (Vec<Saved>, Vec<String>) {
    let rules: Vec<_> = req.endpoint.extract.iter().filter(|r| r.enabled && !r.name.trim().is_empty()).collect();
    let mut saved = Vec::with_capacity(rules.len());
    let mut secrets = Vec::new();
    for rule in rules {
        let name = rule.name.trim().to_owned();
        let result = match (outcome.status, req.environment.as_deref()) {
            (_, None) => Err("no active environment to save into".to_owned()),
            (None, _) => Err("no response".to_owned()),
            (Some(status), Some(env)) => match extract::pick(rule, status, &outcome.response_headers, &outcome.body) {
                Ok(value) if rule.target == ExtractTarget::Secret => store
                    .set_secret(env, &name, Some(value.clone()))
                    .await
                    .map(|()| value)
                    .map_err(|e| format!("{e:#}")),
                other => other,
            },
        };
        let (value, error) = match result {
            Ok(v) if rule.target == ExtractTarget::Secret => {
                secrets.push(v);
                (None, None)
            }
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        saved.push(Saved { name, target: rule.target, value, error });
    }
    (saved, secrets)
}

async fn compile(store: &WorkspaceStore, req: &TryRequest, mask: bool) -> Result<CompiledRequest, ApiError> {
    let workspace = store.workspace();
    let files = store.files_for(&req.endpoint).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    CompiledRequest::compile_with(&req.endpoint, &workspace, &store.secrets(), &files, req.environment.as_deref(), mask)
        .map_err(|e| ApiError::BadRequest(e.to_string()))
}
