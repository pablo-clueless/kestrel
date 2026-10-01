//! Which workspace a request acts on. See `model::workspaces`.
//!
//! Accounts off: the `X-Kestrel-Workspace` id, which is the only credential.
//! Accounts on: the signed-in user's workspace. The header (or `?workspace=`) may name one of the
//! user's own workspaces; naming any other is a 404, as if it didn't exist.

use std::sync::Arc;

use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use super::AppState;
use crate::{auth::AuthedUser, error::ApiError, model::store::WorkspaceStore};

pub const WORKSPACE_HEADER: &str = "x-kestrel-workspace";

/// The caller's workspace. `EventSource` can't set headers, so `?workspace=` works too.
pub struct Scope {
    pub id: Uuid,
    pub store: Arc<WorkspaceStore>,
}

impl FromRequestParts<AppState> for Scope {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let from_query = || parts.uri.query()?.split('&').find_map(|pair| pair.strip_prefix("workspace="));
        let raw = parts.headers.get(WORKSPACE_HEADER).and_then(|v| v.to_str().ok()).or_else(from_query);
        // Parsing as a UUID also keeps the id safe to use in a schema name.
        let requested = raw
            .map(|s| Uuid::try_parse(s).map_err(|_| ApiError::BadRequest("invalid X-Kestrel-Workspace header".into())))
            .transpose()?;

        let id = if state.accounts.enabled {
            // Set by the session middleware, which has already turned away requests without one.
            let user = parts.extensions.get::<AuthedUser>().ok_or(ApiError::Unauthorized("sign in first"))?;
            state.accounts.workspace_for(user, requested).await?
        } else {
            requested.ok_or_else(|| ApiError::BadRequest("missing X-Kestrel-Workspace header".into()))?
        };
        let store = state.workspaces.get(id).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
        Ok(Self { id, store })
    }
}
