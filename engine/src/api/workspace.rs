use std::{borrow::Cow, collections::HashSet, time::Duration};

use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::Deserialize;

use super::AppState;
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
    },
    redact::Redactor,
    template::request::CompiledRequest,
};

const SEND_BODY_BYTES: usize = 256 * 1024;
const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn get(State(state): State<AppState>) -> Json<WorkspaceResponse> {
    Json(WorkspaceResponse { workspace: state.store.workspace(), secret_keys: state.store.secret_keys() })
}

pub async fn put(State(state): State<AppState>, Json(workspace): Json<Workspace>) -> Result<StatusCode, ApiError> {
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
    state.store.save_workspace(workspace).map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_secret(
    State(state): State<AppState>,
    Json(req): Json<SetSecretRequest>,
) -> Result<StatusCode, ApiError> {
    if req.environment.trim().is_empty() || req.key.trim().is_empty() {
        return Err(ApiError::BadRequest("environment and key are required".into()));
    }
    state
        .store
        .set_secret(req.environment.trim(), req.key.trim(), req.value)
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
    State(state): State<AppState>,
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
    let store = state.store.clone();
    let id = tokio::task::spawn_blocking(move || store.save_file(bytes))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    Ok(Json(FileRef { id, name, content_type, size }))
}

/// The request as it would be sent, with secrets masked.
pub async fn render(State(state): State<AppState>, Json(req): Json<TryRequest>) -> Result<Json<SentRequest>, ApiError> {
    let compiled = compile(&state, &req, true)?;
    let r = compiled.render().map_err(ApiError::BadRequest)?;
    let body = r.body_text().map(Cow::into_owned);
    Ok(Json(SentRequest { method: r.method, url: r.url.to_string(), headers: r.headers, body }))
}

/// The "try it" button: one request, full (redacted) response. Then the endpoint's extract rules run
/// against the unredacted response: secrets are stored here, variables are returned for the UI to save.
pub async fn send(State(state): State<AppState>, Json(req): Json<TryRequest>) -> Result<Json<SendResponse>, ApiError> {
    let compiled = compile(&state, &req, false)?;
    let rendered = compiled.render().map_err(ApiError::BadRequest)?;
    let opts = ClientOptions { keep_alive: true, follow_redirects: true, confirmed_hosts: state.hosts.list() };
    let target = client::connect(&rendered.url, &opts).await.map_err(ApiError::BadRequest)?;
    let timeout = req.timeout_ms.map_or(DEFAULT_SEND_TIMEOUT, |ms| Duration::from_millis(ms.into()));
    let outcome = client::execute(&target.client, &rendered, timeout.min(state.config.caps.max_timeout)).await;

    let (saved, new_secrets) = run_extracts(&state, &req, &outcome);
    // Hide secrets saved just now too, e.g. the token in a login response.
    let secret_values: Vec<String> = compiled.secret_values.iter().cloned().chain(new_secrets).collect();
    let redactor = Redactor::new(compiled.api_key_header.as_deref(), &secret_values, Some(&rendered));
    let mut sample = sample::build(&rendered, &outcome, &redactor, SEND_BODY_BYTES);
    // The draft endpoint carries its own `expect`; schema definitions come from its saved collection.
    let contract =
        crate::contract::for_endpoint(&state.store.workspace(), &req.endpoint).map_err(ApiError::BadRequest)?;
    if let (Some(contract), Some(status)) = (contract, outcome.status) {
        let check = contract.check(status, &outcome.body);
        sample.contract = Some(crate::contract::ContractCheck { message: redactor.text(&check.message), ..check });
    }
    Ok(Json(SendResponse { sample, saved }))
}

/// Applies the enabled extract rules. Returns each rule's outcome and the secret values stored.
fn run_extracts(state: &AppState, req: &TryRequest, outcome: &Outcome) -> (Vec<Saved>, Vec<String>) {
    let rules: Vec<_> = req.endpoint.extract.iter().filter(|r| r.enabled && !r.name.trim().is_empty()).collect();
    let mut saved = Vec::with_capacity(rules.len());
    let mut secrets = Vec::new();
    for rule in rules {
        let name = rule.name.trim().to_owned();
        let result = match (outcome.status, req.environment.as_deref()) {
            (_, None) => Err("no active environment to save into".to_owned()),
            (None, _) => Err("no response".to_owned()),
            (Some(status), Some(env)) => {
                extract::pick(rule, status, &outcome.response_headers, &outcome.body).and_then(|value| {
                    if rule.target == ExtractTarget::Secret {
                        state.store.set_secret(env, &name, Some(value.clone())).map_err(|e| format!("{e:#}"))?;
                    }
                    Ok(value)
                })
            }
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

fn compile(state: &AppState, req: &TryRequest, mask: bool) -> Result<CompiledRequest, ApiError> {
    let workspace = state.store.workspace();
    CompiledRequest::compile_with(
        &req.endpoint,
        &workspace,
        &state.store.secrets(),
        &*state.store,
        req.environment.as_deref(),
        mask,
    )
    .map_err(|e| ApiError::BadRequest(e.to_string()))
}
