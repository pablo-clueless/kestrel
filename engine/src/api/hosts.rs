//! "I own or am authorised to test this host." Required before load-type tests against anything that
//! doesn't resolve to loopback. Held for the engine's lifetime; the UI asks again after a restart.

use std::{collections::BTreeSet, sync::RwLock};

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use ts_rs::TS;

use super::AppState;
use crate::error::ApiError;

#[derive(Default)]
pub struct ConfirmedHosts(RwLock<BTreeSet<String>>);

impl ConfirmedHosts {
    pub fn list(&self) -> Vec<String> {
        self.0.read().unwrap().iter().cloned().collect()
    }

    fn insert(&self, host: String) {
        self.0.write().unwrap().insert(host);
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConfirmHostRequest {
    pub host: String,
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<String>> {
    Json(state.hosts.list())
}

pub async fn confirm(
    State(state): State<AppState>,
    Json(req): Json<ConfirmHostRequest>,
) -> Result<StatusCode, ApiError> {
    let host = req.host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains('/') || host.contains(char::is_whitespace) {
        return Err(ApiError::BadRequest("host must be a bare hostname, e.g. api.example.com".into()));
    }
    tracing::info!("host confirmed for load testing: {host}");
    state.hosts.insert(host);
    Ok(StatusCode::NO_CONTENT)
}
