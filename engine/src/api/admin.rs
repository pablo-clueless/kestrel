//! `/api/admin/*`: the admin pages (HANDOFF → Accounts → Admin). Every route needs a session whose
//! email is in `KESTREL_ADMIN_EMAILS`; what each does is in `auth::admin`.

use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
};
use uuid::Uuid;

use super::{AppState, auth::SignedIn};
use crate::{
    auth::{
        AuthedUser, SessionsEnded,
        admin::{AdminOverview, AdminSettings, AdminUser, AdminWorkspace, SetDisabledRequest},
        teams::MemberInfo,
    },
    error::ApiError,
};

/// A signed-in admin of this engine. Anyone else gets a 403 (accounts off: the usual 404).
pub struct Admin(AuthedUser);

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let signed = SignedIn::from_request_parts(parts, state).await?;
        if !state.accounts.is_admin(&signed.user) {
            return Err(ApiError::Forbidden("only this engine's admins can do that (KESTREL_ADMIN_EMAILS)"));
        }
        Ok(Self(signed.user))
    }
}

pub async fn overview(State(state): State<AppState>, Admin(admin): Admin) -> Result<Json<AdminOverview>, ApiError> {
    Ok(Json(state.accounts.admin_overview(&admin, state.runs.running()).await?))
}

pub async fn settings(State(state): State<AppState>, _: Admin) -> Json<AdminSettings> {
    Json(AdminSettings::of(&state.config))
}

pub async fn users(State(state): State<AppState>, Admin(admin): Admin) -> Result<Json<Vec<AdminUser>>, ApiError> {
    Ok(Json(state.accounts.admin_users(&admin).await?))
}

pub async fn verify_email(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.admin_verify_email(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_two_factor(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.admin_reset_two_factor(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn sign_out(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionsEnded>, ApiError> {
    let ended = state.accounts.admin_sign_out(&admin, id).await?;
    Ok(Json(SessionsEnded { ended: ended as u32 }))
}

pub async fn set_disabled(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<SetDisabledRequest>,
) -> Result<StatusCode, ApiError> {
    state.accounts.admin_set_disabled(&admin, id, req.disabled).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the account, and any workspace it was alone in: their runs are stopped and their cached
/// stores dropped, as when a workspace is deleted on its own.
pub async fn delete_user(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    for workspace in state.accounts.admin_delete_user(&admin, id).await? {
        state.runs.cancel_workspace(workspace);
        state.workspaces.forget(workspace);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn workspaces(State(state): State<AppState>, _: Admin) -> Result<Json<Vec<AdminWorkspace>>, ApiError> {
    Ok(Json(state.accounts.admin_workspaces().await?))
}

pub async fn members(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<MemberInfo>>, ApiError> {
    Ok(Json(state.accounts.admin_members(&admin, id).await?))
}

/// Deletes any workspace and everything in it, as its own admins can.
pub async fn delete_workspace(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.admin_check_workspace(id).await?;
    state.runs.cancel_workspace(id);
    state.accounts.delete_workspace(&admin, id).await?;
    state.workspaces.forget(id);
    Ok(StatusCode::NO_CONTENT)
}

/// The admin pages and the account's own profile actions, against the real router and database.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Request, header},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::{TOKEN_HEADER, router},
        config::Config,
        db::{Db, test::require_db},
    };

    const TOKEN: &str = "test-token";
    const HOST: &str = "127.0.0.1:7070";
    const PASSWORD: &str = "correct horse battery";
    const BOSS: &str = "boss@example.com";

    fn app(db: &Arc<Db>) -> Router {
        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.auth_enabled = true;
        config.admin_emails = vec![BOSS.into()];
        config.public_url = Some("http://localhost:3000".into());
        router(AppState::new(config, Arc::clone(db)))
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri).header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN);
        if let Some(cookie) = cookie {
            req = req.header(header::COOKIE, cookie);
        }
        let req = match body {
            Some(body) => {
                let body = body.to_string();
                req.header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, body.len())
                    .body(Body::from(body))
                    .unwrap()
            }
            None => req.body(Body::empty()).unwrap(),
        };
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// Signs up or in; the cookie to send back.
    async fn sign(app: &Router, path: &str, email: &str) -> String {
        let req = Request::post(path)
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "email": email, "password": PASSWORD }).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path} {email}");
        res.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned()
    }

    async fn me(app: &Router, cookie: &str) -> Value {
        send(app, "GET", "/api/auth/me", Some(cookie), None).await.1
    }

    async fn user_id(app: &Router, cookie: &str) -> String {
        me(app, cookie).await["user"]["id"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn only_listed_admins_reach_the_admin_pages() {
        let db = require_db!();
        let app = app(&db);
        let boss = sign(&app, "/api/auth/signup", BOSS).await;
        let joe = sign(&app, "/api/auth/signup", "joe@example.com").await;

        assert_eq!(me(&app, &boss).await["user"]["admin"], json!(true));
        assert_eq!(me(&app, &joe).await["user"]["admin"], json!(false));
        for path in ["/api/admin/overview", "/api/admin/users", "/api/admin/workspaces", "/api/admin/settings"] {
            assert_eq!(send(&app, "GET", path, Some(&joe), None).await.0, StatusCode::FORBIDDEN, "{path}");
            assert_eq!(send(&app, "GET", path, None, None).await.0, StatusCode::UNAUTHORIZED, "{path}");
        }

        let (status, users) = send(&app, "GET", "/api/admin/users", Some(&boss), None).await;
        assert_eq!(status, StatusCode::OK);
        let emails: Vec<&str> = users.as_array().unwrap().iter().map(|u| u["email"].as_str().unwrap()).collect();
        assert_eq!(emails, ["joe@example.com", BOSS], "newest first");
        assert_eq!(users[1]["you"], json!(true));

        let (_, overview) = send(&app, "GET", "/api/admin/overview", Some(&boss), None).await;
        assert_eq!((overview["users"].as_u64(), overview["workspaces"].as_u64()), (Some(2), Some(2)));
        assert_eq!(overview["recentSignups"].as_array().unwrap().len(), 2);

        let (_, settings) = send(&app, "GET", "/api/admin/settings", Some(&boss), None).await;
        assert_eq!(settings["adminEmails"], json!([BOSS]));
        assert!(!settings.to_string().contains(TOKEN), "the token stays out of the settings");
    }

    #[tokio::test]
    async fn a_disabled_account_is_signed_out_and_cant_sign_in_until_enabled() {
        let db = require_db!();
        let app = app(&db);
        let boss = sign(&app, "/api/auth/signup", BOSS).await;
        let joe = sign(&app, "/api/auth/signup", "joe@example.com").await;
        let joe_id = user_id(&app, &joe).await;
        let boss_id = user_id(&app, &boss).await;

        let disabled = |on: bool| json!({ "disabled": on });
        let path = format!("/api/admin/users/{joe_id}/disabled");
        assert_eq!(send(&app, "PUT", &path, Some(&boss), Some(disabled(true))).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, "GET", "/api/workspaces", Some(&joe), None).await.0, StatusCode::UNAUTHORIZED);
        let login = json!({ "email": "joe@example.com", "password": PASSWORD });
        let (status, body) = send(&app, "POST", "/api/auth/login", None, Some(login.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let wrong = json!({ "email": "joe@example.com", "password": "not the password" });
        assert_eq!(
            send(&app, "POST", "/api/auth/login", None, Some(wrong)).await.0,
            StatusCode::UNAUTHORIZED,
            "a wrong password doesn't learn that the account is disabled"
        );

        assert_eq!(send(&app, "PUT", &path, Some(&boss), Some(disabled(false))).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, "POST", "/api/auth/login", None, Some(login)).await.0, StatusCode::OK);

        // Admins can't lock themselves (or another admin) out from here.
        let own = format!("/api/admin/users/{boss_id}/disabled");
        assert_eq!(send(&app, "PUT", &own, Some(&boss), Some(disabled(true))).await.0, StatusCode::BAD_REQUEST);
        let own = format!("/api/admin/users/{boss_id}");
        assert_eq!(send(&app, "DELETE", &own, Some(&boss), None).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn deleting_an_account_takes_its_own_workspaces_but_not_a_shared_one_it_alone_runs() {
        let db = require_db!();
        let app = app(&db);
        let boss = sign(&app, "/api/auth/signup", BOSS).await;
        let joe = sign(&app, "/api/auth/signup", "joe@example.com").await;
        let joe_id = user_id(&app, &joe).await;

        // Joe runs a team workspace Boss is in, as a writer.
        let (_, team) = send(&app, "POST", "/api/workspaces", Some(&joe), Some(json!({ "name": "Team" }))).await;
        let team_id = team["id"].as_str().unwrap().to_owned();
        let invite = json!({ "email": BOSS, "role": "write" });
        let (_, created) =
            send(&app, "POST", &format!("/api/workspaces/{team_id}/invites"), Some(&joe), Some(invite)).await;
        let token = created["link"].as_str().unwrap().split("token=").nth(1).unwrap().to_owned();
        let (status, _) = send(&app, "POST", "/api/invites/accept", Some(&boss), Some(json!({ "token": token }))).await;
        assert_eq!(status, StatusCode::OK);

        let wrong = json!({ "password": "not the password" });
        assert_eq!(
            send(&app, "POST", "/api/auth/delete-account", Some(&joe), Some(wrong)).await.0,
            StatusCode::BAD_REQUEST
        );
        let right = json!({ "password": PASSWORD });
        let (status, body) = send(&app, "POST", "/api/auth/delete-account", Some(&joe), Some(right.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("\"Team\""), "{body}");
        let (status, _) = send(&app, "DELETE", &format!("/api/admin/users/{joe_id}"), Some(&boss), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "the admin pages keep the same rule");

        // Once Boss is an admin too, Joe can go; his own workspace goes with him, the team stays.
        let promote = json!({ "role": "admin" });
        let path = format!("/api/workspaces/{team_id}/members/{}", user_id(&app, &boss).await);
        assert_eq!(send(&app, "PUT", &path, Some(&joe), Some(promote)).await.0, StatusCode::NO_CONTENT);
        let before = send(&app, "GET", "/api/admin/workspaces", Some(&boss), None).await.1.as_array().unwrap().len();
        assert_eq!(
            send(&app, "POST", "/api/auth/delete-account", Some(&joe), Some(right)).await.0,
            StatusCode::NO_CONTENT
        );

        let (_, workspaces) = send(&app, "GET", "/api/admin/workspaces", Some(&boss), None).await;
        let workspaces = workspaces.as_array().unwrap();
        assert_eq!(workspaces.len(), before - 1, "Joe's own workspace is gone");
        let team = workspaces.iter().find(|w| w["id"] == json!(team_id)).expect("the team stays");
        assert_eq!(team["members"], json!(1));
        let login = json!({ "email": "joe@example.com", "password": PASSWORD });
        assert_eq!(send(&app, "POST", "/api/auth/login", None, Some(login)).await.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn display_names_and_signing_out_other_devices() {
        let db = require_db!();
        let app = app(&db);
        let laptop = sign(&app, "/api/auth/signup", "ada@example.com").await;
        let phone = sign(&app, "/api/auth/login", "ada@example.com").await;

        let name = |n: &str| json!({ "name": n });
        assert_eq!(
            send(&app, "PUT", "/api/auth/profile", Some(&laptop), Some(name("  Ada  "))).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(me(&app, &phone).await["user"]["name"], json!("Ada"));
        let long = "a".repeat(81);
        assert_eq!(
            send(&app, "PUT", "/api/auth/profile", Some(&laptop), Some(name(&long))).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(&app, "PUT", "/api/auth/profile", Some(&laptop), Some(name(""))).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(me(&app, &phone).await["user"]["name"], Value::Null);

        let (status, ended) = send(&app, "DELETE", "/api/auth/sessions", Some(&laptop), None).await;
        assert_eq!((status, ended["ended"].as_u64()), (StatusCode::OK, Some(1)));
        assert_eq!(send(&app, "GET", "/api/workspaces", Some(&phone), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, "GET", "/api/workspaces", Some(&laptop), None).await.0, StatusCode::OK);
    }
}
