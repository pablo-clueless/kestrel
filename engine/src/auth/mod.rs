//! Accounts (HANDOFF → Accounts, A1–A3): sign-up, sign-in, sessions, and the emailed links for
//! verification and password reset. Off by default (`KESTREL_AUTH=off`), in which case nothing here
//! runs and a browser's workspace id is its only credential, as before.

pub mod admin;
pub mod mail;
pub mod password;
pub mod profile;
pub mod rate_limit;
pub mod session;
pub mod teams;
pub mod totp;
pub mod two_factor;

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    config::Config,
    db::{
        Db,
        accounts::{CreateUser, EmailPurpose},
        teams::Role,
    },
    error::ApiError,
};
use mail::Mailer;
use rate_limit::RateLimiter;
use teams::WorkspaceInfo;

/// How long an emailed link works.
const VERIFY_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
const RESET_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// What the UI routes are called; links in emails point at them.
const VERIFY_PATH: &str = "/verify-email";
const RESET_PATH: &str = "/reset-password";
/// The same answer for an unknown, used, expired or wrong-purpose token: which one it was doesn't
/// help anyone.
const BAD_LINK: &str = "this link is invalid or has expired";

/// The user a request is acting as, put in the request's extensions by the session middleware.
#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
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
    /// Whether the user has followed a verification (or password reset) link.
    pub email_verified: bool,
    /// Their display name, if they've set one. The UI shows the email otherwise.
    pub name: Option<String>,
    /// Listed in `KESTREL_ADMIN_EMAILS`: may use the admin pages.
    pub admin: bool,
}

/// `GET /api/auth/me`, and what signing in returns.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeResponse {
    pub auth: AuthMode,
    /// Whether new accounts can be created from the UI.
    pub signup: bool,
    /// Whether this engine can send email (`KESTREL_SMTP_HOST`), so verification and password
    /// reset are available.
    pub mail: bool,
    /// The signed-in user; `None` when signed out or when auth is off.
    pub user: Option<UserInfo>,
    /// The workspace the UI should use (it sends this as `X-Kestrel-Workspace`): the one it asked
    /// for if the user is still a member, else their default.
    pub workspace_id: Option<Uuid>,
    /// Every workspace the user belongs to, with their role in each. Empty when signed out or off.
    pub workspaces: Vec<WorkspaceInfo>,
}

/// `PUT /api/auth/profile`.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileRequest {
    /// Up to 80 characters; empty clears it.
    pub name: String,
}

/// `POST /api/auth/delete-account`: the password, and with two-factor on, a code too.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteAccountRequest {
    pub password: String,
    #[ts(optional)]
    pub code: Option<String>,
}

/// `DELETE /api/auth/sessions`: how many other sessions were signed out.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionsEnded {
    pub ended: u32,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /api/auth/password-reset`: email a reset link, if there's an account.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// `POST /api/auth/password-reset/confirm`: the token from the link, and the new password.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PasswordResetConfirm {
    pub token: String,
    pub new_password: String,
}

/// `POST /api/auth/verify-email`: the token from the link.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// One signed-in device or browser, for the Security tab.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionInfo {
    /// Opaque; pass it back to revoke the session. (It's derived from the token's hash, which
    /// can't be turned back into a token.)
    pub id: String,
    /// The session making this request.
    pub current: bool,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    #[ts(type = "number")]
    pub last_seen_at_ms: i64,
    #[ts(type = "number")]
    pub expires_at_ms: i64,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
}

/// Where a sign-in attempt came from, for rate limiting and the session record.
pub struct Client<'a> {
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    /// The page's `Origin` (already checked against the allowlist by the guard). Links in emails
    /// point there unless `KESTREL_PUBLIC_URL` is set.
    pub origin: Option<&'a str>,
}

pub struct Accounts {
    db: Arc<Db>,
    pub enabled: bool,
    pub signup_open: bool,
    by_email: RateLimiter,
    by_ip: RateLimiter,
    password_changes: RateLimiter,
    /// Two-factor codes tried per account, at sign-in or in settings, whatever the challenge: a
    /// 6-digit code has a million values, so guesses must stay few.
    two_factor_attempts: RateLimiter,
    /// `None` when `KESTREL_SMTP_HOST` is unset: verification and reset are then unavailable.
    mailer: Option<Arc<Mailer>>,
    public_url: Option<String>,
    /// Emails sent to one address, whoever asks: a few per quarter hour, so the reset form can't be
    /// used to flood someone's inbox.
    mail_per_address: RateLimiter,
    /// Invites made per workspace per hour, emailed or not.
    invites: RateLimiter,
    /// `KESTREL_ADMIN_EMAILS`, lowercased.
    admin_emails: Vec<String>,
}

/// What the password step of signing in came to.
pub enum LoginOutcome {
    /// Signed in: the session token, and who's signed in.
    SignedIn(String, MeResponse),
    /// The account has two-factor on: a code must follow, with this challenge.
    TwoFactor(String),
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
            password_changes: RateLimiter::per_minute(5),
            two_factor_attempts: RateLimiter::per_minute(10),
            mailer: config.smtp.as_ref().and_then(|smtp| match Mailer::smtp(smtp) {
                Ok(mailer) => Some(Arc::new(mailer)),
                // `serve` builds the mailer first and refuses to start if it fails, so this is
                // only reached by other commands, which don't send email.
                Err(err) => {
                    tracing::error!("email is off: {err:#}");
                    None
                }
            }),
            public_url: config.public_url.clone(),
            mail_per_address: RateLimiter::new(3, Duration::from_secs(15 * 60)),
            invites: RateLimiter::new(20, Duration::from_secs(60 * 60)),
            admin_emails: config.admin_emails.clone(),
        }
    }

    /// Whether `user` may use the admin pages. Never with accounts off: there's nobody to check.
    pub fn is_admin(&self, user: &AuthedUser) -> bool {
        self.enabled && self.admin_emails.contains(&user.email)
    }

    /// Sends through `mailer` instead of the configured one. Tests read what was sent.
    #[cfg(test)]
    pub fn with_mailer(mut self, mailer: Arc<Mailer>) -> Self {
        self.mailer = Some(mailer);
        self
    }

    pub fn me_off(&self) -> MeResponse {
        MeResponse {
            auth: AuthMode::Off,
            signup: false,
            mail: false,
            user: None,
            workspace_id: None,
            workspaces: Vec::new(),
        }
    }

    pub fn me_signed_out(&self) -> MeResponse {
        MeResponse {
            auth: AuthMode::On,
            signup: self.signup_open,
            mail: self.mailer.is_some(),
            user: None,
            workspace_id: None,
            workspaces: Vec::new(),
        }
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
            // Sign-up signs you in straight away, so it can't hide that the address is taken the
            // way the reset form does; keep it vague and point at signing in.
            CreateUser::EmailTaken => {
                return Err(ApiError::BadRequest("can't create an account with that email; try signing in".into()));
            }
        };
        // Best effort: the account works either way, and the Account tab can send it again.
        if let Some(mailer) = &self.mailer {
            match self.link_base(client.origin) {
                Ok(base) => {
                    let (db, mailer, email) = (Arc::clone(&self.db), Arc::clone(mailer), email.clone());
                    tokio::spawn(async move {
                        if let Err(err) = send_verification(&db, &mailer, user, &email, &base).await {
                            tracing::warn!("couldn't send the verification email to {email}: {err:#}");
                        }
                    });
                }
                Err(err) => tracing::warn!("no verification email for {email}: {err}"),
            }
        }
        let user = AuthedUser { id: user, email, email_verified: false, name: None };
        self.start_session(user, client).await
    }

    pub async fn login(&self, req: AuthRequest, client: Client<'_>) -> Result<LoginOutcome, ApiError> {
        let email = normalize_email(&req.email).map_err(|_| ApiError::Unauthorized(BAD_CREDENTIALS))?;
        self.rate_limit(&email, client.ip)?;
        let user = self.db.user_by_email(&email).await.map_err(internal)?;
        let (id, stored) = match user {
            Some(u) => (Some((u.id, u.email_verified, u.two_factor, u.disabled)), Some(u.password_hash)),
            None => (None, None),
        };
        let outdated = stored.as_deref().is_some_and(password::needs_rehash);
        if !password::verify(req.password.clone(), stored).await {
            return Err(ApiError::Unauthorized(BAD_CREDENTIALS));
        }
        let (id, email_verified, two_factor, disabled) = id.expect("verify only succeeds for a known user");
        // Only said once the password is right, so it doesn't reveal which accounts exist.
        if disabled {
            return Err(ApiError::Forbidden("this account has been disabled; ask whoever runs this engine"));
        }
        // The one moment the password is known: upgrade a hash made with older parameters.
        // Best effort, since the sign-in itself has succeeded either way.
        if outdated {
            let upgraded = async { self.db.set_password_hash(id, &password::hash(req.password).await?).await };
            if let Err(err) = upgraded.await {
                tracing::warn!("couldn't rehash the password of {email}: {err:#}");
            }
        }
        if two_factor {
            return Ok(LoginOutcome::TwoFactor(self.two_factor_challenge(id).await?));
        }
        self.db.ensure_user_workspace(id, req.workspace).await.map_err(internal)?;
        let name = self.db.user_by_id(id).await.map_err(internal)?.and_then(|(_, _, name)| name);
        let (token, me) = self.start_session(AuthedUser { id, email, email_verified, name }, client).await?;
        Ok(LoginOutcome::SignedIn(token, me))
    }

    /// Emails a password reset link if `email` has an account. The answer is the same either way,
    /// and the lookup and sending happen after it's sent, so neither the response nor its timing
    /// says whether the account exists.
    pub async fn request_password_reset(&self, req: PasswordResetRequest, client: Client<'_>) -> Result<(), ApiError> {
        let mailer = Arc::clone(self.mailer()?);
        let email = normalize_email(&req.email)?;
        if let Some(ip) = client.ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        self.mail_per_address.check(&email).map_err(ApiError::TooManyRequests)?;
        let base = self.link_base(client.origin)?;
        let db = Arc::clone(&self.db);
        tokio::spawn(async move {
            let sent = async {
                let Some(user) = db.user_by_email(&email).await? else { return Ok(false) };
                let token = session::new_token();
                db.create_email_token(&session::hash(&token), user.id, EmailPurpose::Reset, RESET_LIFETIME).await?;
                mailer.send(mail::password_reset(&email, &format!("{base}{RESET_PATH}?token={token}"))).await?;
                anyhow::Ok(true)
            };
            match sent.await {
                Ok(true) => tracing::info!("password reset link sent to {email}"),
                Ok(false) => {}
                Err(err) => tracing::warn!("couldn't send a password reset link to {email}: {err:#}"),
            }
        });
        Ok(())
    }

    /// Sets a new password from a reset link. Every session ends, including any the person who
    /// asked for the reset doesn't know about; they sign in again with the new password.
    pub async fn confirm_password_reset(&self, req: PasswordResetConfirm, client: Client<'_>) -> Result<(), ApiError> {
        if let Some(ip) = client.ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        let token_hash = link_token(&req.token)?;
        password::check_policy(&req.new_password).map_err(ApiError::BadRequest)?;
        let hash = password::hash(req.new_password).await.map_err(internal)?;
        match self.db.reset_password(&token_hash, &hash).await.map_err(internal)? {
            Some(user) => {
                tracing::info!("password reset for user {user}; all of their sessions ended");
                Ok(())
            }
            None => Err(ApiError::BadRequest(BAD_LINK.into())),
        }
    }

    /// Marks the email verified from a verification link. Works signed in or not, so the link can
    /// be opened on another device.
    pub async fn verify_email(&self, req: VerifyEmailRequest, client: Client<'_>) -> Result<(), ApiError> {
        if let Some(ip) = client.ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        let token_hash = link_token(&req.token)?;
        match self.db.verify_email(&token_hash).await.map_err(internal)? {
            Some(_) => Ok(()),
            None => Err(ApiError::BadRequest(BAD_LINK.into())),
        }
    }

    /// Sends the signed-in user a new verification link. Waits for the send, so the UI can say
    /// whether it worked.
    pub async fn resend_verification(&self, user: &AuthedUser, client: Client<'_>) -> Result<(), ApiError> {
        let mailer = self.mailer()?;
        if user.email_verified {
            return Err(ApiError::BadRequest("your email is already verified".into()));
        }
        self.mail_per_address.check(&user.email).map_err(ApiError::TooManyRequests)?;
        let base = self.link_base(client.origin)?;
        send_verification(&self.db, mailer, user.id, &user.email, &base).await.map_err(|err| {
            tracing::warn!("couldn't send the verification email to {}: {err:#}", user.email);
            ApiError::Internal("couldn't send the email; try again in a few minutes".into())
        })
    }

    fn mailer(&self) -> Result<&Arc<Mailer>, ApiError> {
        self.mailer.as_ref().ok_or(ApiError::NotFound("email isn't set up on this engine (KESTREL_SMTP_HOST)"))
    }

    /// Where links in emails point: `KESTREL_PUBLIC_URL`, else the page that asked.
    fn link_base(&self, origin: Option<&str>) -> Result<String, ApiError> {
        self.public_url
            .clone()
            .or_else(|| origin.map(|o| o.trim_end_matches('/').to_owned()))
            .ok_or_else(|| ApiError::Internal("can't tell where the UI is for the link; set KESTREL_PUBLIC_URL".into()))
    }

    /// Changes `user`'s password after checking the current one, and signs out every other session
    /// (`keep` is the token of the session making the change). Attempts are rate limited per
    /// account, so a stolen session can't be used to guess the password. That's a separate budget
    /// from sign-in, so signing in on a few devices doesn't block a password change.
    pub async fn change_password(
        &self,
        user: &AuthedUser,
        req: ChangePasswordRequest,
        keep: &str,
    ) -> Result<u64, ApiError> {
        self.password_changes.check(&user.id.to_string()).map_err(ApiError::TooManyRequests)?;
        let stored = self.db.password_hash(user.id).await.map_err(internal)?;
        if !password::verify(req.current_password, stored).await {
            return Err(ApiError::BadRequest("current password is incorrect".into()));
        }
        password::check_policy(&req.new_password).map_err(ApiError::BadRequest)?;
        let hash = password::hash(req.new_password).await.map_err(internal)?;
        let ended = self.db.change_password(user.id, &hash, &session::hash(keep)).await.map_err(internal)?;
        tracing::info!("password changed for {}; {ended} other session(s) signed out", user.email);
        Ok(ended)
    }

    /// `user`'s live sessions, with the one making this request marked `current`.
    pub async fn sessions(&self, user: &AuthedUser, current_token: &str) -> Result<Vec<SessionInfo>, ApiError> {
        let current = session::hash(current_token);
        let rows = self.db.sessions_of(user.id).await.map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|row| SessionInfo {
                current: row.id_hash == current,
                id: hex(&row.id_hash),
                created_at_ms: row.created_at_ms,
                last_seen_at_ms: row.last_seen_at_ms,
                expires_at_ms: row.expires_at_ms,
                user_agent: row.user_agent,
                ip: row.ip,
            })
            .collect())
    }

    /// Signs out one of `user`'s sessions by its id from [`Self::sessions`]. Anyone else's session,
    /// or a malformed id, is a 404.
    pub async fn revoke_session(&self, user: &AuthedUser, id: &str) -> Result<(), ApiError> {
        const NOT_FOUND: ApiError = ApiError::NotFound("session not found");
        let id_hash = unhex(id).ok_or(NOT_FOUND)?;
        match self.db.delete_session_of(user.id, &id_hash).await.map_err(internal)? {
            true => Ok(()),
            false => Err(NOT_FOUND),
        }
    }

    /// Deletes expired sessions, email links and invites now, then once a day. They're already refused when
    /// used; this only keeps the tables from growing.
    pub fn spawn_session_sweeper(self: &Arc<Self>) {
        let accounts = Arc::clone(self);
        tokio::spawn(async move {
            let mut every_day = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
            loop {
                every_day.tick().await;
                match accounts.db.delete_expired_sessions().await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!("deleted {n} expired session(s)"),
                    Err(err) => tracing::warn!("couldn't delete expired sessions: {err:#}"),
                }
                if let Err(err) = accounts.db.delete_expired_email_tokens().await {
                    tracing::warn!("couldn't delete expired email links: {err:#}");
                }
                if let Err(err) = accounts.db.delete_expired_invites().await {
                    tracing::warn!("couldn't delete expired invites: {err:#}");
                }
                if let Err(err) = accounts.db.delete_expired_login_challenges().await {
                    tracing::warn!("couldn't delete expired sign-in challenges: {err:#}");
                }
            }
        });
    }

    pub(super) async fn start_session(
        &self,
        user: AuthedUser,
        client: Client<'_>,
    ) -> Result<(String, MeResponse), ApiError> {
        let token = session::new_token();
        self.db
            .create_session(&session::hash(&token), user.id, session::LIFETIME, client.user_agent, client.ip)
            .await
            .map_err(internal)?;
        let me = self.me(&user, None).await?;
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
        Ok(Some(AuthedUser {
            id: found.user_id,
            email: found.email,
            email_verified: found.email_verified,
            name: found.name,
        }))
    }

    /// Who's signed in and their workspaces. `requested` (the UI's current choice) is kept if they're
    /// still a member of it; otherwise, e.g. after being removed from it, they're sent to their default
    /// rather than shown an error.
    pub async fn me(&self, user: &AuthedUser, requested: Option<Uuid>) -> Result<MeResponse, ApiError> {
        // A user always has one (made at sign-up), but repair rather than fail if not.
        let default = self.db.ensure_user_workspace(user.id, None).await.map_err(internal)?;
        let workspaces = self.workspaces(user).await?;
        let workspace = requested.filter(|id| workspaces.iter().any(|w| w.id == *id)).unwrap_or(default);
        Ok(MeResponse {
            auth: AuthMode::On,
            signup: self.signup_open,
            mail: self.mailer.is_some(),
            user: Some(UserInfo {
                id: user.id,
                email: user.email.clone(),
                email_verified: user.email_verified,
                name: user.name.clone(),
                admin: self.is_admin(user),
            }),
            workspace_id: Some(workspace),
            workspaces,
        })
    }

    /// The workspace a signed-in request acts on, and the user's role in it: `requested` if the user
    /// is a member of it (a workspace they aren't a member of is a 404, as if it didn't exist), else
    /// their default.
    pub async fn workspace_for(&self, user: &AuthedUser, requested: Option<Uuid>) -> Result<(Uuid, Role), ApiError> {
        let id = match requested {
            Some(id) => id,
            None => self.db.ensure_user_workspace(user.id, None).await.map_err(internal)?,
        };
        Ok((id, self.role(user, id).await?))
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

/// Makes a verification link for `user` and emails it.
async fn send_verification(db: &Db, mailer: &Mailer, user: Uuid, email: &str, base: &str) -> anyhow::Result<()> {
    let token = session::new_token();
    db.create_email_token(&session::hash(&token), user, EmailPurpose::Verify, VERIFY_LIFETIME).await?;
    mailer.send(mail::verification(email, &format!("{base}{VERIFY_PATH}?token={token}"))).await
}

/// The stored hash for a token from a link. Tokens are 64 hex characters (like session tokens);
/// anything else can't be one, so it's refused without a database lookup.
fn link_token(token: &str) -> Result<Vec<u8>, ApiError> {
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(BAD_LINK.into()));
    }
    Ok(session::hash(&token.to_ascii_lowercase()))
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
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
