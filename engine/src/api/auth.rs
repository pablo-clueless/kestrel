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
        AuthRequest, AuthedUser, ChangePasswordRequest, Client, DeleteAccountRequest, LoginOutcome, MeResponse,
        PasswordResetConfirm, PasswordResetRequest, ProfileRequest, SessionInfo, SessionsEnded, VerifyEmailRequest,
        session,
        two_factor::{
            RecoveryCodes, TwoFactorChallenge, TwoFactorCode, TwoFactorConfirm, TwoFactorLogin, TwoFactorSetup,
            TwoFactorSetupRequest, TwoFactorStatus,
        },
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
            // The code step of signing in: the password step passed, but there's no session yet.
            | "/auth/login/2fa"
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
    pub(super) fn client(&self) -> Client<'_> {
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
    match state.accounts.login(req, caller.client()).await? {
        LoginOutcome::SignedIn(token, me) => Ok(signed_in(&token, caller.https, me)),
        // No cookie yet: the code step sets it.
        LoginOutcome::TwoFactor(challenge) => {
            Ok(Json(TwoFactorChallenge { two_factor_challenge: challenge }).into_response())
        }
    }
}

/// The code step of signing in, for accounts with two-factor on.
pub async fn login_two_factor(
    State(state): State<AppState>,
    caller: Caller,
    Json(req): Json<TwoFactorLogin>,
) -> Result<Response, ApiError> {
    accounts_on(&state)?;
    let (token, me) = state.accounts.complete_two_factor(req, caller.client()).await?;
    Ok(signed_in(&token, caller.https, me))
}

pub async fn two_factor_status(
    State(state): State<AppState>,
    signed: SignedIn,
) -> Result<Json<TwoFactorStatus>, ApiError> {
    Ok(Json(state.accounts.two_factor_status(&signed.user).await?))
}

pub async fn two_factor_setup(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<TwoFactorSetupRequest>,
) -> Result<Json<TwoFactorSetup>, ApiError> {
    Ok(Json(state.accounts.two_factor_setup(&signed.user, req).await?))
}

pub async fn two_factor_enable(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<TwoFactorCode>,
) -> Result<Json<RecoveryCodes>, ApiError> {
    Ok(Json(state.accounts.two_factor_enable(&signed.user, req).await?))
}

pub async fn two_factor_disable(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<TwoFactorConfirm>,
) -> Result<StatusCode, ApiError> {
    state.accounts.two_factor_disable(&signed.user, req).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<TwoFactorConfirm>,
) -> Result<Json<RecoveryCodes>, ApiError> {
    Ok(Json(state.accounts.regenerate_recovery_codes(&signed.user, req).await?))
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
    pub(super) user: AuthedUser,
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

/// Signs out every session but this one.
pub async fn sign_out_others(State(state): State<AppState>, signed: SignedIn) -> Result<Json<SessionsEnded>, ApiError> {
    let ended = state.accounts.sign_out_others(&signed.user, &signed.token).await?;
    Ok(Json(SessionsEnded { ended: ended as u32 }))
}

pub async fn set_profile(
    State(state): State<AppState>,
    signed: SignedIn,
    Json(req): Json<ProfileRequest>,
) -> Result<StatusCode, ApiError> {
    state.accounts.set_name(&signed.user, &req.name).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes the signed-in account (and the workspaces only it was in, whose runs are stopped and
/// stores dropped), and clears the cookie: its session went with it.
pub async fn delete_account(
    State(state): State<AppState>,
    caller: Caller,
    signed: SignedIn,
    Json(req): Json<DeleteAccountRequest>,
) -> Result<Response, ApiError> {
    for workspace in state.accounts.delete_account(&signed.user, req).await? {
        state.runs.cancel_workspace(workspace);
        state.workspaces.forget(workspace);
    }
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, session::clear_cookie(caller.https))]).into_response())
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
    // The UI's current workspace, kept if the user is still a member of it. Malformed is as good as none.
    let requested = headers.get(super::WORKSPACE_HEADER).and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok());
    if !state.accounts.enabled {
        return Ok(Json(state.accounts.me_off()));
    }
    let user = match session::from_headers(&headers) {
        Some(token) => state.accounts.user_for(&token).await?,
        None => None,
    };
    Ok(Json(match user {
        Some(user) => state.accounts.me(&user, requested).await?,
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
        assert_eq!(send(&app, call("PUT", "/api/workspace", Some(&alice), None, Some(saved))).await.0, StatusCode::OK);
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

    /// Shared workspaces: invites `email` to `ws` as `role` and returns the token from the link.
    async fn invite(app: &Router, owner: &str, ws: &str, email: &str, role: &str) -> String {
        let body = json!({ "email": email, "role": role });
        let mut req = post(&format!("/api/workspaces/{ws}/invites"), Some(owner), body);
        // The link points back at the page that asked, as browsers say in `Origin`.
        req.headers_mut().insert(header::ORIGIN, "http://localhost:7070".parse().unwrap());
        let (status, _, created) = send(app, req).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        created["link"].as_str().unwrap().rsplit_once("token=").unwrap().1.to_owned()
    }

    fn accept(cookie: &str, token: &str) -> Request<Body> {
        post("/api/invites/accept", Some(cookie), json!({ "token": token }))
    }

    /// Shared workspaces: a `write` member works in the admin's workspace; a `read` one only looks.
    #[tokio::test]
    async fn write_members_edit_and_read_members_only_look() {
        let db = require_db!();
        let app = app(&db, true);
        let (owner, me) = sign(&app, "/api/auth/signup", "owner@example.com", None).await;
        let ws = me["workspaceId"].as_str().unwrap().to_owned();
        let (editor, _) = sign(&app, "/api/auth/signup", "ed@example.com", None).await;
        let (viewer, _) = sign(&app, "/api/auth/signup", "vi@example.com", None).await;

        let token = invite(&app, &owner, &ws, "Ed@Example.com", "write").await;
        let (status, _, joined) = send(&app, accept(&editor, &token)).await;
        assert_eq!((status, joined["workspaceId"].as_str()), (StatusCode::OK, Some(ws.as_str())), "{joined}");
        let token = invite(&app, &owner, &ws, "vi@example.com", "read").await;
        assert_eq!(send(&app, accept(&viewer, &token)).await.0, StatusCode::OK);

        // The editor's `me` lists both workspaces and keeps the one the UI asked for.
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", Some(&editor), Some(&ws), None)).await;
        assert_eq!(me["workspaceId"].as_str(), Some(ws.as_str()));
        let roles: Vec<_> = me["workspaces"].as_array().unwrap().iter().map(|w| w["role"].clone()).collect();
        assert_eq!(roles, [json!("admin"), json!("write")]);
        assert_eq!(me["workspaces"][1]["name"], "owner's workspace");
        assert_eq!(me["workspaces"][1]["members"], 3);

        let saved = json!({ "environments": [{ "name": "shared", "vars": {} }] });
        let put = |cookie: &str| call("PUT", "/api/workspace", Some(cookie), Some(&ws), Some(saved.clone()));
        assert_eq!(send(&app, put(&editor)).await.0, StatusCode::OK, "editors save");
        let (status, _, body) = send(&app, put(&viewer)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body["error"].as_str().unwrap().contains("read access"), "{body}");

        let (status, _, read) = send(&app, call("GET", "/api/workspace", Some(&viewer), Some(&ws), None)).await;
        assert_eq!((status, &read["workspace"]["environments"][0]["name"]), (StatusCode::OK, &json!("shared")));
        assert_eq!(send(&app, call("GET", "/api/runs", Some(&viewer), Some(&ws), None)).await.0, StatusCode::OK);
        let secret = json!({ "environment": "shared", "key": "token", "value": "abc" });
        let run = json!({ "kind": "fake", "durationMs": 60000 });
        for (method, path, body) in [
            ("PUT", "/api/secrets", secret),
            ("POST", "/api/runs", run.clone()),
            ("POST", "/api/hosts/confirm", json!({ "host": "example.com" })),
        ] {
            let status = send(&app, call(method, path, Some(&viewer), Some(&ws), Some(body))).await.0;
            assert_eq!(status, StatusCode::FORBIDDEN, "viewer {method} {path}");
        }

        // A run the editor starts is visible to everyone in the workspace, and stoppable by the owner.
        let (status, _, started) = send(&app, call("POST", "/api/runs", Some(&editor), Some(&ws), Some(run))).await;
        assert_eq!(status, StatusCode::CREATED);
        let run_id = started["runId"].as_str().unwrap();
        let (_, _, runs) = send(&app, call("GET", "/api/runs", Some(&viewer), Some(&ws), None)).await;
        assert_eq!(runs[0]["runId"].as_str(), Some(run_id));
        let stop = call("DELETE", &format!("/api/runs/{run_id}"), Some(&owner), None, None);
        assert_eq!(send(&app, stop).await.0, StatusCode::ACCEPTED);

        // Downgrading the editor takes effect on their next request.
        let member = |m: &Value| me["user"]["id"] == m["userId"];
        let (_, _, list) =
            send(&app, call("GET", &format!("/api/workspaces/{ws}/members"), Some(&editor), None, None)).await;
        let ed_id = list["members"].as_array().unwrap().iter().find(|m| member(m)).unwrap()["userId"].clone();
        assert_eq!(list["invites"], json!([]), "only admins see invites");
        let demote = call(
            "PUT",
            &format!("/api/workspaces/{ws}/members/{}", ed_id.as_str().unwrap()),
            Some(&owner),
            None,
            Some(json!({ "role": "read" })),
        );
        assert_eq!(send(&app, demote).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, put(&editor)).await.0, StatusCode::FORBIDDEN);
    }

    /// Shared workspaces: a link works once, only for the address it was sent to, until it expires
    /// or is withdrawn; re-inviting replaces the old link.
    #[tokio::test]
    async fn invite_links_are_single_use_and_bound_to_their_email() {
        let db = require_db!();
        let app = app(&db, true);
        let (owner, me) = sign(&app, "/api/auth/signup", "owner@example.com", None).await;
        let ws = me["workspaceId"].as_str().unwrap().to_owned();
        let (ada, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (eve, _) = sign(&app, "/api/auth/signup", "eve@example.com", None).await;

        let first = invite(&app, &owner, &ws, "ada@example.com", "write").await;
        let second = invite(&app, &owner, &ws, "ada@example.com", "read").await;
        let (status, _, body) = send(&app, accept(&ada, &first)).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("this invite is invalid or has expired")),
            "replaced"
        );
        let (status, _, body) = send(&app, accept(&eve, &second)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "someone else's invite: {body}");
        assert_eq!(send(&app, accept(&ada, &second)).await.0, StatusCode::OK, "still good for its addressee");
        assert_eq!(send(&app, accept(&ada, &second)).await.0, StatusCode::BAD_REQUEST, "single use");
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", Some(&ada), Some(&ws), None)).await;
        assert_eq!(me["workspaces"][1]["role"], "read", "the newer invite's role");

        let body = json!({ "email": "ada@example.com", "role": "write" });
        let (status, _, err) = send(&app, post(&format!("/api/workspaces/{ws}/invites"), Some(&owner), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "already a member: {err}");
        // Admins can be invited too: a workspace can have several.
        invite(&app, &owner, &ws, "second-admin@example.com", "admin").await;

        // Withdrawn and expired invites stop working; the admins' list shows only live ones.
        let withdrawn = invite(&app, &owner, &ws, "eve@example.com", "write").await;
        let (_, _, list) =
            send(&app, call("GET", &format!("/api/workspaces/{ws}/members"), Some(&owner), None, None)).await;
        let id = list["invites"][0]["id"].as_str().unwrap().to_owned();
        let revoke = call("DELETE", &format!("/api/workspaces/{ws}/invites/{id}"), Some(&owner), None, None);
        assert_eq!(send(&app, revoke).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, accept(&eve, &withdrawn)).await.0, StatusCode::BAD_REQUEST);
        let expiring = invite(&app, &owner, &ws, "eve@example.com", "write").await;
        let (_, _, list) =
            send(&app, call("GET", &format!("/api/workspaces/{ws}/members"), Some(&owner), None, None)).await;
        db.expire_invite(list["invites"][0]["id"].as_str().unwrap().parse().unwrap()).await;
        assert_eq!(send(&app, accept(&eve, &expiring)).await.0, StatusCode::BAD_REQUEST, "expired");
        let (_, _, list) =
            send(&app, call("GET", &format!("/api/workspaces/{ws}/members"), Some(&owner), None, None)).await;
        let emails: Vec<_> = list["invites"].as_array().unwrap().iter().map(|i| i["email"].clone()).collect();
        assert_eq!(emails, [json!("second-admin@example.com")], "only the live invite");
        assert_eq!(db.delete_expired_invites().await.unwrap(), 1);
    }

    /// Shared workspaces: only admins manage members; there can be several, and always at least one;
    /// members may leave; outsiders see nothing.
    #[tokio::test]
    async fn admins_manage_members_and_one_always_remains() {
        let db = require_db!();
        let app = app(&db, true);
        let (owner, me) = sign(&app, "/api/auth/signup", "owner@example.com", None).await;
        let ws = me["workspaceId"].as_str().unwrap().to_owned();
        let owner_id = me["user"]["id"].as_str().unwrap().to_owned();
        let (ed, ed_me) = sign(&app, "/api/auth/signup", "ed@example.com", None).await;
        let ed_id = ed_me["user"]["id"].as_str().unwrap().to_owned();
        let (outsider, _) = sign(&app, "/api/auth/signup", "out@example.com", None).await;
        let token = invite(&app, &owner, &ws, "ed@example.com", "write").await;
        send(&app, accept(&ed, &token)).await;

        let members = format!("/api/workspaces/{ws}/members");
        assert_eq!(send(&app, call("GET", &members, Some(&outsider), None, None)).await.0, StatusCode::NOT_FOUND);
        let invite_new = |cookie: &str| {
            let mut req = post(
                &format!("/api/workspaces/{ws}/invites"),
                Some(cookie),
                json!({ "email": "new@example.com", "role": "read" }),
            );
            req.headers_mut().insert(header::ORIGIN, "http://localhost:7070".parse().unwrap());
            req
        };
        assert_eq!(send(&app, invite_new(&ed)).await.0, StatusCode::FORBIDDEN, "write members can't invite");
        let rename = |cookie: &str| {
            call("PUT", &format!("/api/workspaces/{ws}"), Some(cookie), None, Some(json!({ "name": "Payments" })))
        };
        assert_eq!(send(&app, rename(&ed)).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, rename(&owner)).await.0, StatusCode::NO_CONTENT);
        let remove = |cookie: &str, id: &str| call("DELETE", &format!("{members}/{id}"), Some(cookie), None, None);
        let set_role = |cookie: &str, id: &str, role: &str| {
            call("PUT", &format!("{members}/{id}"), Some(cookie), None, Some(json!({ "role": role })))
        };
        assert_eq!(send(&app, remove(&ed, &owner_id)).await.0, StatusCode::FORBIDDEN, "write members can't remove");
        assert_eq!(send(&app, set_role(&ed, &ed_id, "admin")).await.0, StatusCode::FORBIDDEN, "or promote themselves");
        let (status, _, body) = send(&app, remove(&owner, &owner_id)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "the last admin can't leave: {body}");
        assert!(body["error"].as_str().unwrap().contains("at least one admin"), "{body}");
        assert_eq!(send(&app, set_role(&owner, &owner_id, "read")).await.0, StatusCode::BAD_REQUEST, "or step down");

        let (_, _, list) = send(&app, call("GET", &members, Some(&ed), None, None)).await;
        assert_eq!(list["members"][0]["role"], "admin");
        assert_eq!(
            (list["members"][1]["email"].as_str(), list["members"][1]["you"].as_bool()),
            (Some("ed@example.com"), Some(true))
        );
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", Some(&ed), Some(&ws), None)).await;
        assert_eq!(me["workspaces"][1]["name"], "Payments");
        assert_eq!(me["workspaces"][1]["adminEmails"], json!(["owner@example.com"]));

        // A second admin can do everything the first can, and then the first can step down.
        assert_eq!(send(&app, set_role(&owner, &ed_id, "admin")).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, invite_new(&ed)).await.0, StatusCode::CREATED, "the new admin invites");
        assert_eq!(send(&app, set_role(&owner, &owner_id, "read")).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, set_role(&owner, &ed_id, "read")).await.0, StatusCode::FORBIDDEN, "no longer an admin");
        assert_eq!(send(&app, remove(&ed, &ed_id)).await.0, StatusCode::BAD_REQUEST, "now ed is the last admin");

        // Leaving: the next request there is a 404, and `me` falls back to the default workspace.
        assert_eq!(send(&app, remove(&owner, &owner_id)).await.0, StatusCode::NO_CONTENT);
        assert_eq!(
            send(&app, call("GET", "/api/workspace", Some(&owner), Some(&ws), None)).await.0,
            StatusCode::NOT_FOUND
        );
        let (_, _, me) = send(&app, call("GET", "/api/auth/me", Some(&owner), Some(&ws), None)).await;
        assert_ne!(me["workspaceId"].as_str(), Some(ws.as_str()));
        assert_eq!(me["workspaces"].as_array().unwrap().len(), 1);
    }
    /// Shared workspaces: creating extra workspaces, and deleting one with everything in it.
    #[tokio::test]
    async fn workspaces_are_created_and_deleted_by_their_admins() {
        let db = require_db!();
        let app = app(&db, true);
        let (owner, me) = sign(&app, "/api/auth/signup", "owner@example.com", None).await;
        let personal = me["workspaceId"].as_str().unwrap().to_owned();
        let (ed, _) = sign(&app, "/api/auth/signup", "ed@example.com", None).await;

        let delete =
            |cookie: &str, id: &str| call("DELETE", &format!("/api/workspaces/{id}"), Some(cookie), None, None);
        assert_eq!(send(&app, delete(&owner, &personal)).await.0, StatusCode::BAD_REQUEST, "not the only one");
        let blank = post("/api/workspaces", Some(&owner), json!({ "name": "  " }));
        assert_eq!(send(&app, blank).await.0, StatusCode::BAD_REQUEST);
        let (status, _, team) = send(&app, post("/api/workspaces", Some(&owner), json!({ "name": " Team " }))).await;
        assert_eq!(
            (status, team["name"].as_str(), team["role"].as_str()),
            (StatusCode::CREATED, Some("Team"), Some("admin"))
        );
        let team_id = team["id"].as_str().unwrap().to_owned();

        let saved = json!({ "environments": [{ "name": "team-only", "vars": {} }] });
        let put = call("PUT", "/api/workspace", Some(&owner), Some(&team_id), Some(saved));
        assert_eq!(send(&app, put).await.0, StatusCode::OK);
        let (_, _, ws) = send(&app, call("GET", "/api/workspace", Some(&owner), Some(&personal), None)).await;
        assert_eq!(ws["workspace"]["environments"], json!([]), "separate data");
        let token = invite(&app, &owner, &team_id, "ed@example.com", "write").await;
        send(&app, accept(&ed, &token)).await;
        let run = json!({ "kind": "fake", "durationMs": 60000 });
        assert_eq!(
            send(&app, call("POST", "/api/runs", Some(&ed), Some(&team_id), Some(run))).await.0,
            StatusCode::CREATED
        );

        assert_eq!(send(&app, delete(&ed, &team_id)).await.0, StatusCode::FORBIDDEN, "write members can't delete");
        assert_eq!(send(&app, delete(&owner, &team_id)).await.0, StatusCode::NO_CONTENT);
        for cookie in [&owner, &ed] {
            let status = send(&app, call("GET", "/api/workspace", Some(cookie), Some(&team_id), None)).await.0;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM pg_namespace WHERE nspname = $1")
            .bind(db.schema_of(team_id.parse().unwrap()))
            .fetch_optional(db.pool())
            .await
            .unwrap();
        assert_eq!(exists, None, "the schema is gone");
        assert_eq!(send(&app, delete(&owner, &personal)).await.0, StatusCode::BAD_REQUEST, "the last one stays");
    }

    /// Shared workspaces: with email set up, the invite is emailed as well as returned.
    #[tokio::test]
    async fn invites_are_emailed_when_mail_is_on() {
        let db = require_db!();
        let (app, mailer) = app_with_mail(&db, Some("https://kestrel.test"));
        let (owner, me) = sign(&app, "/api/auth/signup", "owner@example.com", None).await;
        let ws = me["workspaceId"].as_str().unwrap().to_owned();
        let body = json!({ "email": "ada@example.com", "role": "read" });
        let (_, _, created) = send(&app, post(&format!("/api/workspaces/{ws}/invites"), Some(&owner), body)).await;
        assert_eq!(created["emailed"], true);
        let link = created["link"].as_str().unwrap();
        assert!(link.starts_with("https://kestrel.test/invite?token="), "{link}");
        let sent = mailer.sent();
        let email = sent.iter().find(|e| e.to == "ada@example.com").unwrap();
        assert!(
            email.body.contains(link)
                && email.body.contains("owner@example.com")
                && email.body.contains("with read access")
        );
    }

    fn unix_now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }

    /// Signs up `email` and turns two-factor on. Returns the cookie, the base32 secret and the
    /// recovery codes. The code used to turn it on is for the current step, so the next usable one is
    /// the following step's.
    async fn with_two_factor(app: &Router, email: &str) -> (String, String, Vec<String>) {
        let (cookie, _) = sign(app, "/api/auth/signup", email, None).await;
        let (status, _, setup) =
            send(app, post("/api/auth/2fa/setup", Some(&cookie), json!({ "password": "correct horse battery" }))).await;
        assert_eq!(status, StatusCode::OK, "{setup}");
        let secret = setup["secret"].as_str().unwrap().to_owned();
        assert!(setup["otpauthUrl"].as_str().unwrap().starts_with("otpauth://totp/Kestrel:"), "{setup}");
        let code = crate::auth::totp::code_for(&secret, unix_now());
        let (status, _, codes) = send(app, post("/api/auth/2fa/enable", Some(&cookie), json!({ "code": code }))).await;
        assert_eq!(status, StatusCode::OK, "{codes}");
        let codes = codes["codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_owned()).collect();
        (cookie, secret, codes)
    }

    /// The password step; with two-factor on it returns a challenge and no cookie.
    async fn password_step(app: &Router, email: &str) -> String {
        let body = json!({ "email": email, "password": "correct horse battery" });
        let (status, headers, res) = send(app, post("/api/auth/login", None, body)).await;
        assert_eq!(status, StatusCode::OK, "{res}");
        assert!(headers.get(header::SET_COOKIE).is_none(), "no session before the code");
        res["twoFactorChallenge"].as_str().unwrap_or_else(|| panic!("a challenge: {res}")).to_owned()
    }

    fn code_step(challenge: &str, code: &str) -> Request<Body> {
        post("/api/auth/login/2fa", None, json!({ "challenge": challenge, "code": code }))
    }

    /// Two-factor: setup needs the password and a working code; signing in then needs a code or a
    /// recovery code, each usable once; turning it off needs the password and a code.
    #[tokio::test]
    async fn two_factor_sign_in_needs_a_code_each_used_once() {
        let db = require_db!();
        let app = app(&db, true);
        let (cookie, _) = sign(&app, "/api/auth/signup", "ada@example.com", None).await;
        let (_, _, status) = send(&app, call("GET", "/api/auth/2fa", Some(&cookie), None, None)).await;
        assert_eq!(status["enabled"], false);
        let wrong = post("/api/auth/2fa/setup", Some(&cookie), json!({ "password": "not my password" }));
        assert_eq!(send(&app, wrong).await.0, StatusCode::BAD_REQUEST, "setup needs the password");
        let (_, _, setup) =
            send(&app, post("/api/auth/2fa/setup", Some(&cookie), json!({ "password": "correct horse battery" })))
                .await;
        let secret = setup["secret"].as_str().unwrap();
        let enable = |code: &str| post("/api/auth/2fa/enable", Some(&cookie), json!({ "code": code }));
        assert_eq!(send(&app, enable("000000")).await.0, StatusCode::BAD_REQUEST, "a wrong code doesn't turn it on");
        let (_, _, status) = send(&app, call("GET", "/api/auth/2fa", Some(&cookie), None, None)).await;
        assert_eq!(status["enabled"], false, "not until a code matches");
        let (status, _, codes) = send(&app, enable(&crate::auth::totp::code_for(secret, unix_now()))).await;
        assert_eq!(status, StatusCode::OK);
        let codes: Vec<String> =
            codes["codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().into()).collect();
        assert_eq!(codes.len(), 10);
        let (_, _, status) = send(&app, call("GET", "/api/auth/2fa", Some(&cookie), None, None)).await;
        assert_eq!((status["enabled"].as_bool(), status["recoveryCodesLeft"].as_i64()), (Some(true), Some(10)));
        let again = post("/api/auth/2fa/setup", Some(&cookie), json!({ "password": "correct horse battery" }));
        assert_eq!(send(&app, again).await.0, StatusCode::BAD_REQUEST, "can't swap the secret while it's on");

        // Signing in: the password alone gives a challenge, not a session.
        let challenge = password_step(&app, "ada@example.com").await;
        assert_eq!(send(&app, code_step(&challenge, "000000")).await.0, StatusCode::UNAUTHORIZED);
        let next = crate::auth::totp::code_for(secret, unix_now() + 30);
        let (status, headers, me) = send(&app, code_step(&challenge, &next)).await;
        assert_eq!(status, StatusCode::OK, "{me}");
        assert_eq!(me["user"]["email"], "ada@example.com");
        let signed_in = headers[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
        assert_eq!(send(&app, call("GET", "/api/workspace", Some(&signed_in), None, None)).await.0, StatusCode::OK);
        assert_eq!(send(&app, code_step(&challenge, &next)).await.0, StatusCode::UNAUTHORIZED, "challenge used up");
        let challenge = password_step(&app, "ada@example.com").await;
        assert_eq!(send(&app, code_step(&challenge, &next)).await.0, StatusCode::UNAUTHORIZED, "the code was used");

        // A recovery code works once, typed any way.
        let typed = codes[0].replace('-', "").to_uppercase();
        assert_eq!(send(&app, code_step(&challenge, &typed)).await.0, StatusCode::OK);
        let challenge = password_step(&app, "ada@example.com").await;
        assert_eq!(send(&app, code_step(&challenge, &codes[0])).await.0, StatusCode::UNAUTHORIZED, "used up");
    }

    /// Two-factor: new recovery codes replace the old ones; turning it off needs the password and a
    /// code, after which the password alone signs in again. (Its own account: password
    /// confirmations are limited to 5 a minute per account.)
    #[tokio::test]
    async fn two_factor_recovery_codes_rotate_and_turning_it_off_needs_a_code() {
        let db = require_db!();
        let app = app(&db, true);
        let (cookie, _, codes) = with_two_factor(&app, "ada@example.com").await;
        let confirm = |path: &str, password: &str, code: &str| {
            post(path, Some(&cookie), json!({ "password": password, "code": code }))
        };
        let (status, _, fresh) =
            send(&app, confirm("/api/auth/2fa/recovery-codes", "correct horse battery", &codes[1])).await;
        assert_eq!(status, StatusCode::OK, "{fresh}");
        let fresh: Vec<String> =
            fresh["codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().into()).collect();
        let challenge = password_step(&app, "ada@example.com").await;
        assert_eq!(send(&app, code_step(&challenge, &codes[2])).await.0, StatusCode::UNAUTHORIZED, "old codes stop");

        let wrong_password = confirm("/api/auth/2fa/disable", "not my password", &fresh[0]);
        assert_eq!(send(&app, wrong_password).await.0, StatusCode::BAD_REQUEST);
        let wrong_code = confirm("/api/auth/2fa/disable", "correct horse battery", "000000");
        assert_eq!(send(&app, wrong_code).await.0, StatusCode::BAD_REQUEST);
        let off = confirm("/api/auth/2fa/disable", "correct horse battery", &fresh[0]);
        assert_eq!(send(&app, off).await.0, StatusCode::NO_CONTENT);
        sign(&app, "/api/auth/login", "ada@example.com", None).await;
        let (_, _, status) = send(&app, call("GET", "/api/auth/2fa", Some(&cookie), None, None)).await;
        assert_eq!((status["enabled"].as_bool(), status["recoveryCodesLeft"].as_i64()), (Some(false), Some(0)));
    }

    /// Two-factor: a challenge takes 5 wrong codes, then needs the password again; it can't be made up.
    #[tokio::test]
    async fn two_factor_challenges_run_out_and_cant_be_forged() {
        let db = require_db!();
        let app = app(&db, true);
        let (_, secret, _) = with_two_factor(&app, "ada@example.com").await;
        let challenge = password_step(&app, "ada@example.com").await;
        for _ in 0..5 {
            assert_eq!(send(&app, code_step(&challenge, "000000")).await.0, StatusCode::UNAUTHORIZED);
        }
        let right = crate::auth::totp::code_for(&secret, unix_now() + 30);
        let (status, _, body) = send(&app, code_step(&challenge, &right)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "out of attempts, even with the right code");
        assert!(body["error"].as_str().unwrap().contains("password again"), "{body}");
        let forged = crate::auth::session::new_token();
        assert_eq!(send(&app, code_step(&forged, &right)).await.0, StatusCode::UNAUTHORIZED);

        // The CLI's way out for someone who lost everything.
        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.auth_enabled = true;
        Accounts::new(Arc::clone(&db), &config).reset_two_factor("ADA@example.com").await.unwrap();
        sign(&app, "/api/auth/login", "ada@example.com", None).await;
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
