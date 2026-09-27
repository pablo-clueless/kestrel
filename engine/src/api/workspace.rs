use std::{collections::HashSet, time::Duration};

use axum::{Json, extract::State, http::StatusCode};

use super::AppState;
use crate::{
    engine::{
        client::{self, ClientOptions},
        sample,
        types::{SentRequest, Sample},
    },
    error::ApiError,
    model::{SetSecretRequest, TryRequest, Workspace, WorkspaceResponse},
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

/// The request as it would be sent, with secrets masked.
pub async fn render(State(state): State<AppState>, Json(req): Json<TryRequest>) -> Result<Json<SentRequest>, ApiError> {
    let compiled = compile(&state, &req, true)?;
    let r = compiled.render().map_err(ApiError::BadRequest)?;
    Ok(Json(SentRequest { method: r.method, url: r.url.to_string(), headers: r.headers, body: r.body }))
}

/// The "try it" button: one request, full (redacted) response.
pub async fn send(State(state): State<AppState>, Json(req): Json<TryRequest>) -> Result<Json<Sample>, ApiError> {
    let compiled = compile(&state, &req, false)?;
    let rendered = compiled.render().map_err(ApiError::BadRequest)?;
    let opts = ClientOptions { keep_alive: true, follow_redirects: true, confirmed_hosts: state.hosts.list() };
    let target = client::connect(&rendered.url, &opts).await.map_err(ApiError::BadRequest)?;
    let timeout = req.timeout_ms.map_or(DEFAULT_SEND_TIMEOUT, |ms| Duration::from_millis(ms.into()));
    let outcome = client::execute(&target.client, &rendered, timeout.min(state.config.caps.max_timeout)).await;

    let redactor = Redactor::new(compiled.api_key_header.as_deref(), &compiled.secret_values, Some(&rendered));
    let mut sample = sample::build(&rendered, &outcome, &redactor, SEND_BODY_BYTES);
    // The draft endpoint carries its own `expect`; schema definitions come from its saved collection.
    let contract = crate::contract::for_endpoint(&state.store.workspace(), &req.endpoint).map_err(ApiError::BadRequest)?;
    if let (Some(contract), Some(status)) = (contract, outcome.status) {
        let check = contract.check(status, &outcome.body);
        sample.contract = Some(crate::contract::ContractCheck { message: redactor.text(&check.message), ..check });
    }
    Ok(Json(sample))
}

fn compile(state: &AppState, req: &TryRequest, mask: bool) -> Result<CompiledRequest, ApiError> {
    let workspace = state.store.workspace();
    CompiledRequest::compile(&req.endpoint, &workspace, &state.store.secrets(), req.environment.as_deref(), mask)
        .map_err(|e| ApiError::BadRequest(e.to_string()))
}
