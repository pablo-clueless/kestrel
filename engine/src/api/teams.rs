//! `/api/workspaces/*` and `/api/invites/accept`: shared workspaces (HANDOFF → Accounts → Shared
//! workspaces). Every route needs a session; who may do what is decided in `auth::teams`. These act
//! on the workspace named in the path, not the `X-Kestrel-Workspace` one, so the Settings page can
//! manage any of the user's workspaces.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use super::{AppState, auth::Caller, auth::SignedIn};
use crate::{
    auth::teams::{
        AcceptInviteRequest, AcceptInviteResponse, InviteCreated, InviteRequest, MembersResponse, SetRoleRequest,
        WorkspaceInfo, WorkspaceNameRequest,
    },
    error::ApiError,
};

pub async fn list(State(state): State<AppState>, signed: SignedIn) -> Result<Json<Vec<WorkspaceInfo>>, ApiError> {
    Ok(Json(state.accounts.workspaces(&signed.user).await?))
}

pub async fn create(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<WorkspaceNameRequest>,
) -> Result<(StatusCode, Json<WorkspaceInfo>), ApiError> {
    Ok((StatusCode::CREATED, Json(state.accounts.create_workspace(&signed.user, &req.name).await?)))
}

pub async fn rename(
    State(state): State<AppState>,
    signed: SignedIn,
    Path(id): Path<Uuid>,
    Json(req): Json<WorkspaceNameRequest>,
) -> Result<StatusCode, ApiError> {
    state.accounts.rename_workspace(&signed.user, id, &req.name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the workspace and everything in it. Its runs are stopped and its cached store dropped,
/// so nothing keeps serving or writing to it.
pub async fn delete(
    State(state): State<AppState>,
    signed: SignedIn,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.check_delete_workspace(&signed.user, id).await?;
    state.runs.cancel_workspace(id);
    state.accounts.delete_workspace(&signed.user, id).await?;
    state.workspaces.forget(id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn members(
    State(state): State<AppState>,
    signed: SignedIn,
    Path(id): Path<Uuid>,
) -> Result<Json<MembersResponse>, ApiError> {
    Ok(Json(state.accounts.members(&signed.user, id).await?))
}

pub async fn invite(
    State(state): State<AppState>,
    signed: SignedIn,
    caller: Caller,
    Path(id): Path<Uuid>,
    Json(req): Json<InviteRequest>,
) -> Result<(StatusCode, Json<InviteCreated>), ApiError> {
    Ok((StatusCode::CREATED, Json(state.accounts.invite(&signed.user, id, req, caller.client()).await?)))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((id, invite)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.accounts.revoke_invite(&signed.user, id, invite).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_role(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((id, member)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetRoleRequest>,
) -> Result<StatusCode, ApiError> {
    state.accounts.set_role(&signed.user, id, member, req.role).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The owner removing a member, or a member leaving (their own id).
pub async fn remove_member(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((id, member)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.accounts.remove_member(&signed.user, id, member).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn accept_invite(
    State(state): State<AppState>,
    signed: SignedIn,
    caller: Caller,
    Json(req): Json<AcceptInviteRequest>,
) -> Result<Json<AcceptInviteResponse>, ApiError> {
    Ok(Json(state.accounts.accept_invite(&signed.user, &req.token, caller.client()).await?))
}
