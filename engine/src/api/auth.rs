//! `/api/auth/*`, and the middleware that requires a session on every other route when accounts
//! are on (HANDOFF → Accounts). The guard still runs first: the session cookie is in addition to the
//! `X-Kestrel-Token` header, which doubles as the CSRF check.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, Path, Request, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::AppState;
use crate::{
    auth::{
        AuthRequest, AuthedUser, ChangePasswordRequest, Client, MeResponse, PasswordResetConfirm, PasswordResetRequest,
        SessionInfo, VerifyEmailRequest, session,
    },
    error::ApiError,
};

/// Routes that work without a session. Everything else under `/api` needs one when accounts are on.
fn is_public(path: &str) -> bool {
    // Nested routers may or may not see the `/api` prefix; accept both.
    let path = path.strip_prefix("/api").unwrap_or(path);
    matches!(
        path,
        "/health"
            | "/auth/signup"
            | "/auth/login"
            | "/auth/logout"
            | "/auth/me"
            // Emailed links are opened signed out (often on another device).
            | "/auth/password-reset"
            | "/auth/password-reset/confirm"
            | "/auth/verify-email"
    )
}

/// Puts the signed-in user in the request's extensions, or answers 401.
pub async fn require_session(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if !state.accounts.enabled || is_public(req.uri().path()) {
        return next.run(req).await;
    }
    let Some(token) = session::from_headers(req.headers()) else {
        return ApiError::Unauthorized("sign in first").into_response();
    };
    match state.accounts.user_for(&token).await {
        Ok(Some(user)) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        Ok(None) => ApiError::Unauthorized("your session has ended; sign in again").into_response(),
        Err(err) => err.into_response(),
    }
}

/// Who's calling: for rate limits, the session record, whether the cookie can be `Secure`, and
/// where links in emails should point.
pub struct Caller {
    ip: Option<String>,
    user_agent: Option<String>,
    https: bool,
    /// The guard has already refused origins that aren't allowed, so this one is safe to link to.
    origin: Option<String>,
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
        // Forwarded headers are only believed behind a proxy known to set them; otherwise any
        // client could pick its own IP and dodge the per-IP rate limit.
        let trusted = state.config.trusted_proxy;
        let forwarded_ip = trusted
            .then(|| header("fly-client-ip").or_else(|| header("x-forwarded-for")?.split(',').next().map(str::trim)))
            .flatten()
            .filter(|ip| !ip.is_empty())
            .map(str::to_owned);
        let socket_ip = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string());
        Ok(Self {
            ip: forwarded_ip.or(socket_ip),
            user_agent: header(header::USER_AGENT.as_str()).map(|ua| ua.chars().take(256).collect()),
            https: trusted && header("x-forwarded-proto").is_some_and(|p| p.eq_ignore_ascii_case("https")),
            origin: header(header::ORIGIN.as_str()).filter(|o| *o != "null").map(str::to_owned),
        })
    }
}

impl Caller {
    fn client(&self) -> Client<'_> {
        Client { ip: self.ip.as_deref(), user_agent: self.user_agent.as_deref(), origin: self.origin.as_deref() }
    }
}

fn signed_in(token: &str, https: bool, me: MeResponse) -> Response {
    (StatusCode::OK, [(header::SET_COOKIE, session::set_cookie(token, https))], Json(me)).into_response()
}

const ACCOUNTS_OFF: ApiError = ApiError::NotFound("accounts are off on this engine (KESTREL_AUTH=off)");

pub async fn signup(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<AuthRequest>,
) -> Result<Response, ApiError> {
    if !state.accounts.enabled {
        return Err(ACCOUNTS_OFF);
    }
    let (token, me) = state.accounts.signup(req, caller.client()).await?;
    tracing::info!("account created: {}", me.user.as_ref().map_or("", |u| u.email.as_str()));
    Ok(signed_in(&token, caller.https, me))
}

pub async fn login(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<AuthRequest>,
) -> Result<Response, ApiError> {
    if !state.accounts.enabled {
        return Err(ACCOUNTS_OFF);
    }
    let (token, me) = state.accounts.login(req, caller.client()).await?;
    Ok(signed_in(&token, caller.https, me))
}

/// Ends the session (if any) and clears the cookie. Always succeeds, so a stale cookie can't trap
/// anyone in a signed-in UI.
pub async fn logout(State(state): State<AppState>, caller: Caller, headers: HeaderMap) -> Result<Response, ApiError> {
    if state.accounts.enabled
        && let Some(token) = session::from_headers(&headers)
    {
        state.accounts.logout(&token).await?;
    }
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, session::clear_cookie(caller.https))]).into_response())
}

/// The signed-in user and their session token, for the account routes that need both. 404 when
/// accounts are off, 401 without a session (the middleware has normally answered that already).
pub struct SignedIn {
    user: AuthedUser,
    token: String,
}

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if !state.accounts.enabled {
            return Err(ACCOUNTS_OFF);
        }
        let user = parts.extensions.get::<AuthedUser>().cloned().ok_or(ApiError::Unauthorized("sign in first"))?;
        let token = session::from_headers(&parts.headers).ok_or(ApiError::Unauthorized("sign in first"))?;
        Ok(Self { user, token })
    }
}

/// Changes the password; every other session is signed out, this one stays.
pub async fn change_password(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    state.accounts.change_password(&signed.user, req, &signed.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn sessions(State(state): State<AppState>, signed: SignedIn) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    Ok(Json(state.accounts.sessions(&signed.user, &signed.token).await?))
}

/// Signs out one session. Revoking the current one works too; the UI's next call then gets a 401.
pub async fn revoke_session(
    State(state): State<AppState>,
    signed: SignedIn,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.accounts.revoke_session(&signed.user, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn accounts_on(state: &AppState) -> Result<(), ApiError> {
    if state.accounts.enabled { Ok(()) } else { Err(ACCOUNTS_OFF) }
}

/// Emails a reset link if the account exists. Always 204, so it doesn't say whether it does.
pub async fn request_password_reset(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<PasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    accounts_on(&state)?;
    state.accounts.request_password_reset(req, caller.client()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets a new password from a reset link. Every session ends, so the UI sends people to sign in.
pub async fn confirm_password_reset(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<PasswordResetConfirm>,
) -> Result<StatusCode, ApiError> {
    accounts_on(&state)?;
    state.accounts.confirm_password_reset(req, caller.client()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn verify_email(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<VerifyEmailRequest>,
) -> Result<StatusCode, ApiError> {
    accounts_on(&state)?;
    state.accounts.verify_email(req, caller.client()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn resend_verification(
    State(state): State<AppState>,
    caller: Caller,
    signed: SignedIn,
) -> Result<StatusCode, ApiError> {
    state.accounts.resend_verification(&signed.user, caller.client()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Who's signed in and which workspace to use: `auth: "off"` when accounts are off, `user: null`
/// when signed out. Always 200, so the sign-in page can learn whether sign-up is open.
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<MeResponse>, ApiError> {
    if !state.accounts.enabled {
        return Ok(Json(state.accounts.me_off()));
    }
    let user = match session::from_headers(&headers) {
        Some(token) => state.accounts.user_for(&token).await?,
        None => None,
    };
    Ok(Json(match user {
        Some(user) => state.accounts.me(&user).await?,
        None => state.accounts.me_signed_out(),
    }))
}

/// HANDOFF → Accounts → "Tests (A1 is done when these pass)", against the real router and database.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::{
        api::{TOKEN_HEADER, WORKSPACE_HEADER, router},
        auth::{Accounts, mail::Mailer},
        config::Config,
        db::{Db, test::require_db},
    };

    const TOKEN: &str = "test-token";
    const HOST: &str = "127.0.0.1:7070";

    fn app(db: &Arc<Db>, signup_open: bool) -> Router {
        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.auth_enabled = true;
        config.signup_open = signup_open;
        router(AppState::new(config, Arc::clone(db)))
    }

    /// A request as the UI sends it: deployment token, optional session cookie and workspace.
    fn call(
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        workspace: Option<&str>,
        body: Option<Value>,
    ) -> Request<Body> {
        let mut req = Request::builder().method(method).uri(uri).header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN);
        if let Some(cookie) = cookie {
            req = req.header(header::COOKIE, cookie);
        }
        if let Some(ws) = workspace {
            req = req.header(WORKSPACE_HEADER, ws);
        }
        match body {
            Some(body) => {
                let body = body.to_string();
                req.header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, body.len())
                    .body(Body::from(body))
                    .unwrap()
            }
            None => req.body(Body::empty()).unwrap(),
        }
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// Signs up (or in) and returns the cookie to send back and the `me` body.
    async fn sign(app: &Router, path: &str, email: &str, workspace: Option<Uuid>) -> (String, Value) {
        let body = json!({ "email": email, "password": "correct horse battery", "workspace": workspace });
        let (status, headers, me) = send(app, call("POST", path, None, None, Some(body))).await;
        assert_eq!(status, StatusCode::OK, "{path} {email}: {me}");
        let set = headers[header::SET_COOKIE].to_str().unwrap();
        assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax"), "{set}");
        (set.split(';').next().unwrap().to_owned(), me)
    }

    fn token_of(cookie: &str) -> &str {
        cookie.split_once('=').unwrap().1
    }

    #[tokio::test]
    async fn signup_me_logout_and_the_session_check() {
        let db = require_db!();
        let app = app(&db, true);
        let (_, _, anon) = send(&app, call("GET", "/api/auth/me", None, None, None)).await;
        assert_eq!(
            (anon["auth"].as_str(), anon["user"].is_null(), anon["signup"].as_bool()),
            (Some("on"), true, Some(true))
        );
        assert_eq!(
            send(&app, call("GET", "/api/health", None, None, None)).await.0,
            StatusCode::OK,
            "health is public"
        );
        assert_eq!(send(&app, call("GET", "/api/workspace", None, None, None)).await.0, StatusCode::UNAUTHORIZED);
        let import = json!({ "source": { "type": "text", "text": "{}" } });
        assert_eq!(
            send(&app, call("POST", "/api/import", None, None, Some(import))).await.0,
            StatusCode::UNAUTHORIZED,
            "routes without a workspace need a session too"
        );

        let (cookie, me) = sign(&app, "/api/auth/signup", " Ada@Example.com ", None).await;
        assert_eq!(me["user"]["email"], "ada@example.com");
        let ws = me["workspaceId"].as_str().unwrap().to_owned();
        let (status, _, body) = send(&app, call("GET", "/api/auth/me", Some(&cookie), None, None)).await;
        assert_eq!((status, body["workspaceId"].as_str()), (StatusCode::OK, Some(ws.as_str())));
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&cookie), None, None)).await.0, StatusCode::OK);
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&cookie), Some(&ws), None)).await.0, StatusCode::OK);

        let (status, headers, _) = send(&app, call("POST", "/api/auth/logout", Some(&cookie), None, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(headers[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
        let (_, _, after) = send(&app, call("GET", "/api/auth/me", Some(&cookie), None, None)).await;
        assert!(after["user"].is_null(), "signed out: {after}");
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&cookie), None, None)).await.0,
            StatusCode::UNAUTHORIZED
        );

        let (_, again) = sign(&app, "/api/auth/login", "ada@example.com", None).await;
        assert_eq!(again["workspaceId"].as_str(), Some(ws.as_str()), "signing in again lands in the same workspace");
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_email_look_the_same() {
        let db = require_db!();
        let app = app(&db, true);
        sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let attempt = |email: &str| {
            call("POST", "/api/auth/login", None, None, Some(json!({ "email": email, "password": "wrong password" })))
        };
        let wrong = send(&app, attempt("ada@example.com")).await;
        let unknown = send(&app, attempt("nobody@example.com")).await;
        assert_eq!(wrong.0, StatusCode::UNAUTHORIZED);
        assert_eq!((wrong.0, &wrong.2), (unknown.0, &unknown.2));
        assert!(wrong.1.get(header::SET_COOKIE).is_none());
        let taken = json!({ "email": "ADA@example.com", "password": "another long one" });
        assert_eq!(
            send(&app, call("POST", "/api/auth/signup", None, None, Some(taken))).await.0,
            StatusCode::BAD_REQUEST
        );
        let short = json!({ "email": "bob@example.com", "password": "short" });
        assert_eq!(
            send(&app, call("POST", "/api/auth/signup", None, None, Some(short))).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn users_cannot_reach_each_others_workspaces_runs_or_events() {
        let db = require_db!();
        let app = app(&db, true);
        let (alice, a) = sign(&app, "/api/auth/signup", "alice@example.com", None).await;
        let (bob, _) = sign(&app, "/api/auth/signup", "bob@example.com", None).await;
        let alice_ws = a["workspaceId"].as_str().unwrap();

        let saved = json!({ "environments": [{ "name": "alice-only", "vars": {} }] });
        assert_eq!(
            send(&app, call("PUT", "/api/workspace", Some(&alice), None, Some(saved))).await.0,
            StatusCode::NO_CONTENT
        );
        let run = json!({ "kind": "fake", "durationMs": 60000 });
        let (status, _, started) = send(&app, call("POST", "/api/runs", Some(&alice), None, Some(run))).await;
        assert_eq!(status, StatusCode::CREATED);
        let run_id = started["runId"].as_str().unwrap();

        // Bob naming Alice's workspace: as if it didn't exist.
        for (method, path) in [
            ("GET", "/api/workspace".to_owned()),
            ("GET", "/api/runs".to_owned()),
            ("GET", format!("/api/runs/{run_id}/report")),
            ("DELETE", format!("/api/runs/{run_id}")),
        ] {
            let status = send(&app, call(method, &path, Some(&bob), Some(alice_ws), None)).await.0;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        }
        let sse = format!("/api/runs/{run_id}/events?token={TOKEN}&workspace={alice_ws}");
        let req = Request::get(&sse).header(header::HOST, HOST).header(header::COOKIE, &bob).body(Body::empty());
        assert_eq!(send(&app, req.unwrap()).await.0, StatusCode::NOT_FOUND, "SSE too");

        // In his own workspace, none of Alice's data or runs.
        let (_, _, ws) = send(&app, call("GET", "/api/workspace", Some(&bob), None, None)).await;
        assert_eq!(ws["workspace"]["environments"], json!([]));
        let (_, _, runs) = send(&app, call("GET", "/api/runs", Some(&bob), None, None)).await;
        assert_eq!(runs, json!([]));
        let stop = call("DELETE", &format!("/api/runs/{run_id}"), Some(&bob), None, None);
        assert_eq!(send(&app, stop).await.0, StatusCode::NOT_FOUND);

        let stop = call("DELETE", &format!("/api/runs/{run_id}"), Some(&alice), None, None);
        assert_eq!(send(&app, stop).await.0, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn a_cookie_without_the_deployment_token_is_refused() {
        let db = require_db!();
        let app = app(&db, true);
        let (cookie, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        // A cross-site page can make the browser send the cookie, but not this header.
        let req = Request::get("/api/workspace").header(header::HOST, HOST).header(header::COOKIE, &cookie);
        assert_eq!(send(&app, req.body(Body::empty()).unwrap()).await.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn expired_and_revoked_sessions_are_refused() {
        let db = require_db!();
        let app = app(&db, true);
        let (expiring, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (revoked, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;
        let (other, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;

        db.expire_session(&session::hash(token_of(&expiring))).await;
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&expiring), None, None)).await.0,
            StatusCode::UNAUTHORIZED
        );

        send(&app, call("POST", "/api/auth/logout", Some(&revoked), None, None)).await;
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&revoked), None, None)).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&other), None, None)).await.0,
            StatusCode::OK,
            "other sessions are unaffected"
        );
    }

    #[tokio::test]
    async fn too_many_attempts_for_one_email_get_429() {
        let db = require_db!();
        let app = app(&db, true);
        let bad = || {
            call(
                "POST",
                "/api/auth/login",
                None,
                None,
                Some(json!({ "email": "ada@example.com", "password": "nope nope" })),
            )
        };
        for _ in 0..5 {
            assert_eq!(send(&app, bad()).await.0, StatusCode::UNAUTHORIZED);
        }
        let (status, headers, _) = send(&app, bad()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(headers[header::RETRY_AFTER].to_str().unwrap().parse::<u64>().unwrap() >= 1);
    }

    #[tokio::test]
    async fn the_first_sign_in_claims_an_unowned_browser_workspace() {
        let db = require_db!();
        let app = app(&db, true);
        // A browser that used the engine before accounts (or before signing up).
        let browser = Uuid::new_v4();
        db.ensure_workspace(browser).await.unwrap();

        let (_, ada) = sign(&app, "/api/auth/signup", "ada@example.com", Some(browser)).await;
        assert_eq!(ada["workspaceId"].as_str(), Some(browser.to_string().as_str()), "adopted");
        let (_, bob) = sign(&app, "/api/auth/signup", "bob@example.com", Some(browser)).await;
        assert_ne!(bob["workspaceId"].as_str(), Some(browser.to_string().as_str()), "already owned: not adopted");

        // An id that was never used gets a fresh workspace rather than that id.
        let unused = Uuid::new_v4();
        let (_, cy) = sign(&app, "/api/auth/signup", "cy@example.com", Some(unused)).await;
        assert_ne!(cy["workspaceId"].as_str(), Some(unused.to_string().as_str()));
    }

    #[tokio::test]
    async fn closed_sign_up_still_allows_cli_created_accounts() {
        let db = require_db!();
        let app = app(&db, false);
        let body = json!({ "email": "ada@example.com", "password": "correct horse battery" });
        assert_eq!(send(&app, call("POST", "/api/auth/signup", None, None, Some(body))).await.0, StatusCode::FORBIDDEN);
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", None, None, None)).await;
        assert_eq!(me["signup"], false);

        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.auth_enabled = true;
        let password = crate::auth::Accounts::new(Arc::clone(&db), &config).add_user("ada@example.com").await.unwrap();
        let body = json!({ "email": "ada@example.com", "password": password });
        assert_eq!(send(&app, call("POST", "/api/auth/login", None, None, Some(body))).await.0, StatusCode::OK);
    }

    fn login_with(password: &str) -> Request<Body> {
        call("POST", "/api/auth/login", None, None, Some(json!({ "email": "ada@example.com", "password": password })))
    }

    /// A2: needs the current password; signs out every other session, keeps this one.
    #[tokio::test]
    async fn changing_the_password_signs_out_other_sessions() {
        let db = require_db!();
        let app = app(&db, true);
        let (here, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (laptop, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;
        let (phone, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;

        let change = |current: &str, new: &str| {
            let body = json!({ "currentPassword": current, "newPassword": new });
            call("POST", "/api/auth/password", Some(&here), None, Some(body))
        };
        let (status, _, body) = send(&app, change("not my password", "a brand new passphrase")).await;
        assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("current password is incorrect")));
        assert_eq!(
            send(&app, change("correct horse battery", "short")).await.0,
            StatusCode::BAD_REQUEST,
            "policy applies"
        );
        assert_eq!(
            send(&app, change("correct horse battery", "a brand new passphrase")).await.0,
            StatusCode::NO_CONTENT
        );

        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&here), None, None)).await.0, StatusCode::OK);
        for other in [&laptop, &phone] {
            assert_eq!(
                send(&app, call("GET", "/api/workspace", Some(other), None, None)).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(send(&app, login_with("correct horse battery")).await.0, StatusCode::UNAUTHORIZED, "old password");
        assert_eq!(send(&app, login_with("a brand new passphrase")).await.0, StatusCode::OK);

        let no_session =
            json!({ "currentPassword": "a brand new passphrase", "newPassword": "something else entirely" });
        let req = call("POST", "/api/auth/password", None, None, Some(no_session));
        assert_eq!(send(&app, req).await.0, StatusCode::UNAUTHORIZED);
    }

    /// A2: the Security tab's list, and revoking from it, are limited to your own sessions.
    #[tokio::test]
    async fn users_list_and_revoke_only_their_own_sessions() {
        let db = require_db!();
        let app = app(&db, true);
        let (ada, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (ada_laptop, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;
        let (bob, _) = sign(&app, "/api/auth/signup", "bob@example.com", None).await;

        let (status, _, list) = send(&app, call("GET", "/api/auth/sessions", Some(&ada), None, None)).await;
        assert_eq!(status, StatusCode::OK);
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list.iter().filter(|s| s["current"] == true).count(), 1);
        assert!(list.iter().all(|s| s["expiresAtMs"].as_i64() > s["lastSeenAtMs"].as_i64()));
        let laptop_id = list.iter().find(|s| s["current"] == false).unwrap()["id"].as_str().unwrap().to_owned();
        assert!(!laptop_id.contains(token_of(&ada_laptop)), "ids aren't tokens");

        let (_, _, bobs) = send(&app, call("GET", "/api/auth/sessions", Some(&bob), None, None)).await;
        let bob_id = bobs[0]["id"].as_str().unwrap().to_owned();
        for id in [bob_id.as_str(), "not-an-id", &"0".repeat(64)] {
            let req = call("DELETE", &format!("/api/auth/sessions/{id}"), Some(&ada), None, None);
            assert_eq!(send(&app, req).await.0, StatusCode::NOT_FOUND, "{id}");
        }
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&bob), None, None)).await.0, StatusCode::OK);

        let req = call("DELETE", &format!("/api/auth/sessions/{laptop_id}"), Some(&ada), None, None);
        assert_eq!(send(&app, req).await.0, StatusCode::NO_CONTENT);
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&ada_laptop), None, None)).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&ada), None, None)).await.0, StatusCode::OK);
    }

    /// A2: the daily sweep removes expired sessions only.
    #[tokio::test]
    async fn the_sweep_deletes_only_expired_sessions() {
        let db = require_db!();
        let app = app(&db, true);
        let (old, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (live, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;
        db.expire_session(&session::hash(token_of(&old))).await;
        assert_eq!(db.delete_expired_sessions().await.unwrap(), 1);
        assert_eq!(db.delete_expired_sessions().await.unwrap(), 0);
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&live), None, None)).await.0, StatusCode::OK);
    }

    /// A2: a hash made with weaker parameters is replaced at the next successful sign-in.
    #[tokio::test]
    async fn signing_in_upgrades_an_outdated_hash() {
        use argon2::{Algorithm, Argon2, Params, Version, password_hash::PasswordHasher};

        let db = require_db!();
        let app = app(&db, true);
        let (_, me) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let id: Uuid = me["user"]["id"].as_str().unwrap().parse().unwrap();
        let weak = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(8192, 1, 1, None).unwrap())
            .hash_password(b"correct horse battery")
            .unwrap()
            .to_string();
        db.set_password_hash(id, &weak).await.unwrap();

        assert_eq!(send(&app, login_with("correct horse battery")).await.0, StatusCode::OK, "old hash still works");
        let stored = db.password_hash(id).await.unwrap().unwrap();
        assert_ne!(stored, weak);
        assert!(!crate::auth::password::needs_rehash(&stored), "{stored}");
        assert_eq!(send(&app, login_with("correct horse battery")).await.0, StatusCode::OK, "new hash works");
    }

    /// A3: accounts on, with email going to a capture instead of SMTP.
    fn app_with_mail(db: &Arc<Db>, public_url: Option<&str>) -> (Router, Arc<Mailer>) {
        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.auth_enabled = true;
        config.public_url = public_url.map(str::to_owned);
        let mailer = Arc::new(Mailer::capture());
        let mut state = AppState::new(config.clone(), Arc::clone(db));
        state.accounts = Arc::new(Accounts::new(Arc::clone(db), &config).with_mailer(Arc::clone(&mailer)));
        (router(state), mailer)
    }

    /// Waits for the `n`th email to `to` whose link goes to `path` (some are sent in the background)
    /// and returns the token in it.
    async fn link_token(mailer: &Mailer, to: &str, path: &str, n: usize) -> String {
        for _ in 0..200 {
            let links: Vec<String> = mailer
                .sent()
                .iter()
                .filter(|e| e.to == to)
                .filter_map(|e| e.body.lines().find(|l| l.contains(&format!("{path}?token="))).map(str::to_owned))
                .collect();
            if let Some(link) = links.get(n - 1) {
                return link.rsplit_once("token=").unwrap().1.to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("no email #{n} to {to} with a {path} link; sent: {:?}", mailer.sent());
    }

    fn post(uri: &str, cookie: Option<&str>, body: Value) -> Request<Body> {
        call("POST", uri, cookie, None, Some(body))
    }

    /// A3: the reset link sets a new password once, ends every session, and verifies the email.
    #[tokio::test]
    async fn a_reset_link_sets_a_new_password_once_and_signs_out_everywhere() {
        let db = require_db!();
        let (app, mailer) = app_with_mail(&db, Some("https://kestrel.test"));
        let (here, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (laptop, _) = sign(&app, "/api/auth/login", "ada@example.com", None).await;

        for email in ["ada@example.com", "nobody@example.com"] {
            let (status, _, body) = send(&app, post("/api/auth/password-reset", None, json!({ "email": email }))).await;
            assert_eq!(status, StatusCode::NO_CONTENT, "the same answer for {email}: {body}");
        }
        let token = link_token(&mailer, "ada@example.com", "/reset-password", 1).await;
        let sent = mailer.sent();
        assert!(sent.iter().all(|e| e.to != "nobody@example.com"), "no email to an unknown address");
        let reset = sent.iter().find(|e| e.body.contains("/reset-password")).unwrap();
        assert!(reset.body.contains(&format!("https://kestrel.test/reset-password?token={token}")));

        let confirm = |token: &str, password: &str| {
            post("/api/auth/password-reset/confirm", None, json!({ "token": token, "newPassword": password }))
        };
        assert_eq!(send(&app, confirm(&token, "short")).await.0, StatusCode::BAD_REQUEST, "policy applies");
        assert_eq!(send(&app, confirm(&token, "a brand new passphrase")).await.0, StatusCode::NO_CONTENT);
        let (status, _, body) = send(&app, confirm(&token, "and another one again")).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("this link is invalid or has expired")),
            "single use"
        );
        assert_eq!(send(&app, confirm("not-a-token", "a brand new passphrase")).await.0, StatusCode::BAD_REQUEST);

        for session in [&here, &laptop] {
            let status = send(&app, call("GET", "/api/workspace", Some(session), None, None)).await.0;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "every session ended");
        }
        assert_eq!(send(&app, login_with("correct horse battery")).await.0, StatusCode::UNAUTHORIZED);
        let body = json!({ "email": "ada@example.com", "password": "a brand new passphrase" });
        let (status, _, me) = send(&app, post("/api/auth/login", None, body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["user"]["emailVerified"], true, "the link proved the address");
    }

    /// A3: links expire, and each kind only works for its own purpose.
    #[tokio::test]
    async fn links_expire_and_only_work_for_their_own_purpose() {
        let db = require_db!();
        let (app, mailer) = app_with_mail(&db, Some("https://kestrel.test"));
        sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let verify = link_token(&mailer, "ada@example.com", "/verify-email", 1).await;
        send(&app, post("/api/auth/password-reset", None, json!({ "email": "ada@example.com" }))).await;
        let reset = link_token(&mailer, "ada@example.com", "/reset-password", 1).await;

        let new_password = |token: &str| {
            post("/api/auth/password-reset/confirm", None, json!({ "token": token, "newPassword": "a brand new one" }))
        };
        assert_eq!(send(&app, new_password(&verify)).await.0, StatusCode::BAD_REQUEST, "verify link can't reset");
        let verify_with = |token: &str| post("/api/auth/verify-email", None, json!({ "token": token }));
        assert_eq!(send(&app, verify_with(&reset)).await.0, StatusCode::BAD_REQUEST, "reset link can't verify");

        db.expire_email_token(&session::hash(&reset)).await;
        assert_eq!(send(&app, new_password(&reset)).await.0, StatusCode::BAD_REQUEST, "expired");
        assert_eq!(send(&app, verify_with(&verify)).await.0, StatusCode::NO_CONTENT, "still good for its purpose");
        assert_eq!(db.delete_expired_email_tokens().await.unwrap(), 1, "the sweep reclaims the expired one");
    }

    /// A3: a verification email goes out at sign-up; the link works signed out; the Account tab can
    /// ask for another one until the address is verified.
    #[tokio::test]
    async fn sign_up_sends_a_verification_link() {
        let db = require_db!();
        // No KESTREL_PUBLIC_URL: links go back to the page that asked.
        let (app, mailer) = app_with_mail(&db, None);
        let mut req =
            post("/api/auth/signup", None, json!({ "email": "ada@example.com", "password": "correct horse battery" }));
        req.headers_mut().insert(header::ORIGIN, "http://localhost:7070".parse().unwrap());
        let (status, headers, me) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{me}");
        assert_eq!((me["mail"].as_bool(), me["user"]["emailVerified"].as_bool()), (Some(true), Some(false)));
        let cookie = headers[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
        let first = link_token(&mailer, "ada@example.com", "/verify-email", 1).await;
        assert!(mailer.sent()[0].body.contains("http://localhost:7070/verify-email?token="));

        // Asking again, e.g. the first one went to spam; both links work until one is used.
        let mut again = post("/api/auth/verify-email/resend", Some(&cookie), json!({}));
        again.headers_mut().insert(header::ORIGIN, "http://localhost:7070".parse().unwrap());
        assert_eq!(send(&app, again).await.0, StatusCode::NO_CONTENT);
        let second = link_token(&mailer, "ada@example.com", "/verify-email", 2).await;
        assert_ne!(first, second);

        let verify = |token: &str| post("/api/auth/verify-email", None, json!({ "token": token }));
        assert_eq!(send(&app, verify(&second)).await.0, StatusCode::NO_CONTENT, "no session needed");
        assert_eq!(send(&app, verify(&first)).await.0, StatusCode::BAD_REQUEST, "the others stop working");
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", Some(&cookie), None, None)).await;
        assert_eq!(me["user"]["emailVerified"], true);
        let resend = post("/api/auth/verify-email/resend", Some(&cookie), json!({}));
        assert_eq!(send(&app, resend).await.0, StatusCode::BAD_REQUEST, "already verified");
    }

    /// A3: the reset form can't be used to flood an inbox.
    #[tokio::test]
    async fn reset_emails_are_limited_per_address() {
        let db = require_db!();
        let (app, _mailer) = app_with_mail(&db, Some("https://kestrel.test"));
        let ask = || post("/api/auth/password-reset", None, json!({ "email": "ada@example.com" }));
        for _ in 0..3 {
            assert_eq!(send(&app, ask()).await.0, StatusCode::NO_CONTENT);
        }
        let (status, headers, _) = send(&app, ask()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(headers[header::RETRY_AFTER].to_str().unwrap().parse::<u64>().unwrap() > 60);
    }

    /// A3: without SMTP, the email features say so instead of failing quietly.
    #[tokio::test]
    async fn without_smtp_the_email_routes_are_off() {
        let db = require_db!();
        let app = app(&db, true);
        let (cookie, me) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        assert_eq!(me["mail"], false);
        let reset = post("/api/auth/password-reset", None, json!({ "email": "ada@example.com" }));
        assert_eq!(send(&app, reset).await.0, StatusCode::NOT_FOUND);
        let resend = post("/api/auth/verify-email/resend", Some(&cookie), json!({}));
        assert_eq!(send(&app, resend).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn auth_routes_answer_sensibly_when_accounts_are_off() {
        let db = require_db!();
        let app = router(AppState::new(Config::new(7070, TOKEN.into(), false, vec![]), Arc::clone(&db)));
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", None, None, None)).await;
        assert_eq!(me["auth"], "off");
        let body = json!({ "email": "ada@example.com", "password": "correct horse battery" });
        assert_eq!(send(&app, call("POST", "/api/auth/signup", None, None, Some(body))).await.0, StatusCode::NOT_FOUND);
    }
}
