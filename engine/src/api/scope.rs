//! Which workspace a request acts on, and what the caller may do there. See `model::workspaces`.
//!
//! Accounts off: the `X-Kestrel-Workspace` id, which is the only credential; the caller is its admin.
//! Accounts on: the signed-in user's workspace. The header (or `?workspace=`) may name any workspace
//! the user is a member of; naming any other is a 404, as if it didn't exist.
//!
//! Read-only members are refused anything but reading here, in one place, so a new route that changes or sends
//! something is closed to them by default rather than by remembering to check.

use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{Method, request::Parts},
};
use uuid::Uuid;

use super::AppState;
use crate::{auth::AuthedUser, db::teams::Role, error::ApiError, model::store::WorkspaceStore};

pub const WORKSPACE_HEADER: &str = "x-kestrel-workspace";

/// `POST` routes that only read: `read` members may use them.
const READ_ONLY_POSTS: &[&str] = &["/render"];

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

        let (id, role) = if state.accounts.enabled {
            // Set by the session middleware, which has already turned away requests without one.
            let user = parts.extensions.get::<AuthedUser>().ok_or(ApiError::Unauthorized("sign in first"))?;
            state.accounts.workspace_for(user, requested).await?
        } else {
            let id = requested.ok_or_else(|| ApiError::BadRequest("missing X-Kestrel-Workspace header".into()))?;
            (id, Role::Admin)
        };
        if !role.can_edit() && !reads_only(parts) {
            return Err(ApiError::Forbidden(
                "you have read access to this workspace, so you can't change it or send requests; ask an admin for write access",
            ));
        }
        let store = state.workspaces.get(id).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
        // Defence in depth: never serve a store for a workspace other than the one just authorised,
        // whatever a cache bug might hand back.
        if store.id() != id {
            tracing::error!(authorised = %id, served = %store.id(), "workspace store mismatch; refusing the request");
            return Err(ApiError::Internal("workspace mismatch; the request was refused".into()));
        }
        Ok(Self { id, store })
    }
}

/// Whether the request only reads the workspace: `GET`/`HEAD`, or a `POST` known to only read.
fn reads_only(parts: &Parts) -> bool {
    let path = parts.uri.path();
    // Nested routers may or may not see the `/api` prefix; accept both.
    let path = path.strip_prefix("/api").unwrap_or(path);
    matches!(parts.method, Method::GET | Method::HEAD)
        || (parts.method == Method::POST && READ_ONLY_POSTS.contains(&path))
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    #[test]
    fn only_reads_are_open_to_read_members() {
        let reads = |method: &str, uri: &str| {
            reads_only(&Request::builder().method(method).uri(uri).body(()).unwrap().into_parts().0)
        };
        assert!(reads("GET", "/api/workspace"));
        assert!(reads("GET", "/api/runs/x/events?workspace=y"));
        assert!(reads("POST", "/api/render"));
        assert!(reads("POST", "/render"), "nested routers may strip /api");
        for (method, uri) in [
            ("PUT", "/api/workspace"),
            ("POST", "/api/send"),
            ("POST", "/api/runs"),
            ("DELETE", "/api/runs/x"),
            ("POST", "/api/render/x"),
        ] {
            assert!(!reads(method, uri), "{method} {uri}");
        }
    }
}
