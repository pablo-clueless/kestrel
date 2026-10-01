//! Accounts (HANDOFF → Accounts, A1): sign-up, sign-in, sessions. Off by default
//! (`KESTREL_AUTH=off`), in which case nothing here runs and a browser's workspace id is its only
//! credential, as before.

pub mod password;
pub mod rate_limit;
pub mod session;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    config::Config,
    db::{Db, accounts::CreateUser},
    error::ApiError,
};
use rate_limit::RateLimiter;

/// The user a request is acting as, put in the request's extensions by the session middleware.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub id: Uuid,
    pub email: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuthRequest {
    pub email: String,
    pub password: String,
    /// This browser's existing workspace (from `localStorage`). On a first sign-in it becomes the
    /// account's workspace if nobody owns it yet, so the data made before signing up isn't lost.
    #[ts(optional)]
    pub workspace: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AuthMode {
    /// No accounts: the workspace id alone grants access.
    Off,
    On,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UserInfo {
    pub id: Uuid,
    pub email: String,
}

/// `GET /api/auth/me`, and what signing in returns.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeResponse {
    pub auth: AuthMode,
    /// Whether new accounts can be created from the UI.
    pub signup: bool,
    /// The signed-in user; `None` when signed out or when auth is off.
    pub user: Option<UserInfo>,
    /// The workspace the UI should use (it sends this as `X-Kestrel-Workspace`).
    pub workspace_id: Option<Uuid>,
}

/// Where a sign-in attempt came from, for rate limiting and the session record.
pub struct Client<'a> {
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
}

pub struct Accounts {
    db: Arc<Db>,
    pub enabled: bool,
    pub signup_open: bool,
    by_email: RateLimiter,
    by_ip: RateLimiter,
}

/// Same message for a wrong password and an unknown email, so it doesn't reveal who has an account.
const BAD_CREDENTIALS: &str = "invalid email or password";

impl Accounts {
    pub fn new(db: Arc<Db>, config: &Config) -> Self {
        Self {
            db,
            enabled: config.auth_enabled,
            signup_open: config.signup_open,
            by_email: RateLimiter::per_minute(5),
            by_ip: RateLimiter::per_minute(20),
        }
    }

    pub fn me_off(&self) -> MeResponse {
        MeResponse { auth: AuthMode::Off, signup: false, user: None, workspace_id: None }
    }

    pub fn me_signed_out(&self) -> MeResponse {
        MeResponse { auth: AuthMode::On, signup: self.signup_open, user: None, workspace_id: None }
    }

    /// Creates an account and signs it in. Returns the session token and who's signed in.
    pub async fn signup(&self, req: AuthRequest, client: Client<'_>) -> Result<(String, MeResponse), ApiError> {
        if !self.signup_open {
            return Err(ApiError::Forbidden("sign-up is closed; ask whoever runs this engine for an account"));
        }
        let email = normalize_email(&req.email)?;
        self.rate_limit(&email, client.ip)?;
        password::check_policy(&req.password).map_err(ApiError::BadRequest)?;
        let hash = password::hash(req.password).await.map_err(internal)?;
        let user = match self.db.create_user(&email, &hash, req.workspace).await.map_err(internal)? {
            CreateUser::Created(id) => id,
            // Without email verification (A3) the sign-up form can't avoid saying this somehow;
            // keep it vague and point at signing in.
            CreateUser::EmailTaken => {
                return Err(ApiError::BadRequest("can't create an account with that email; try signing in".into()));
            }
        };
        self.start_session(user, email, client).await
    }

    pub async fn login(&self, req: AuthRequest, client: Client<'_>) -> Result<(String, MeResponse), ApiError> {
        let email = normalize_email(&req.email).map_err(|_| ApiError::Unauthorized(BAD_CREDENTIALS))?;
        self.rate_limit(&email, client.ip)?;
        let user = self.db.user_by_email(&email).await.map_err(internal)?;
        let (id, stored) = match user {
            Some(u) => (Some(u.id), Some(u.password_hash)),
            None => (None, None),
        };
        if !password::verify(req.password, stored).await {
            return Err(ApiError::Unauthorized(BAD_CREDENTIALS));
        }
        let id = id.expect("verify only succeeds for a known user");
        self.db.ensure_user_workspace(id, req.workspace).await.map_err(internal)?;
        self.start_session(id, email, client).await
    }

    async fn start_session(
        &self,
        user: Uuid,
        email: String,
        client: Client<'_>,
    ) -> Result<(String, MeResponse), ApiError> {
        let token = session::new_token();
        self.db
            .create_session(&session::hash(&token), user, session::LIFETIME, client.user_agent, client.ip)
            .await
            .map_err(internal)?;
        let me = self.me(&AuthedUser { id: user, email }).await?;
        Ok((token, me))
    }

    pub async fn logout(&self, token: &str) -> Result<(), ApiError> {
        self.db.delete_session(&session::hash(token)).await.map_err(internal)
    }

    /// The signed-in user for a session token, sliding the session's expiry at most once a minute.
    pub async fn user_for(&self, token: &str) -> Result<Option<AuthedUser>, ApiError> {
        let hash = session::hash(token);
        let Some(found) = self.db.session(&hash).await.map_err(internal)? else { return Ok(None) };
        if found.stale {
            self.db.touch_session(&hash, session::LIFETIME).await.map_err(internal)?;
        }
        Ok(Some(AuthedUser { id: found.user_id, email: found.email }))
    }

    pub async fn me(&self, user: &AuthedUser) -> Result<MeResponse, ApiError> {
        let workspace = self.workspace_for(user, None).await?;
        Ok(MeResponse {
            auth: AuthMode::On,
            signup: self.signup_open,
            user: Some(UserInfo { id: user.id, email: user.email.clone() }),
            workspace_id: Some(workspace),
        })
    }

    /// The workspace a signed-in request acts on: `requested` if the user is a member of it (a
    /// workspace they aren't a member of is a 404, as if it didn't exist), else their default.
    pub async fn workspace_for(&self, user: &AuthedUser, requested: Option<Uuid>) -> Result<Uuid, ApiError> {
        match requested {
            Some(id) if self.db.is_member(user.id, id).await.map_err(internal)? => Ok(id),
            Some(_) => Err(ApiError::NotFound("workspace not found")),
            // A user always has one (made at sign-up), but repair rather than fail if not.
            None => self.db.ensure_user_workspace(user.id, None).await.map_err(internal),
        }
    }

    /// Creates an account from the CLI (`engine user add`), even when sign-up is closed. Returns
    /// the generated password.
    pub async fn add_user(&self, email: &str) -> anyhow::Result<String> {
        let email = normalize_email(email).map_err(|e| anyhow::anyhow!("{e}"))?;
        let password = generate_password();
        let hash = password::hash(password.clone()).await?;
        match self.db.create_user(&email, &hash, None).await? {
            CreateUser::Created(_) => Ok(password),
            CreateUser::EmailTaken => anyhow::bail!("an account with {email} already exists"),
        }
    }

    fn rate_limit(&self, email: &str, ip: Option<&str>) -> Result<(), ApiError> {
        if let Some(ip) = ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        self.by_email.check(email).map_err(ApiError::TooManyRequests)
    }
}

/// Trimmed and lowercased. A light shape check only: deliverability is what email verification
/// (A3) is for.
pub fn normalize_email(raw: &str) -> Result<String, ApiError> {
    let email = raw.trim().to_lowercase();
    let valid = email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if valid { Ok(email) } else { Err(ApiError::BadRequest("enter a valid email address".into())) }
}

/// 20 characters from a 54-letter alphabet without look-alikes (~115 bits).
fn generate_password() -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..20).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect()
}

fn internal(err: anyhow::Error) -> ApiError {
    ApiError::Internal(format!("{err:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_emails() {
        assert_eq!(normalize_email("  Ada@Example.COM ").unwrap(), "ada@example.com");
        for bad in ["", "ada", "@example.com", "ada@localhost", "a da@example.com"] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn generated_passwords_pass_the_policy() {
        let pw = generate_password();
        assert!(password::check_policy(&pw).is_ok());
        assert_ne!(pw, generate_password());
    }
}
