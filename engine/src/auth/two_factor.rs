//! Two-factor sign-in (HANDOFF → Accounts → Two-factor): an authenticator app's codes (TOTP), with
//! single-use recovery codes for when the phone is gone. Off until the user turns it on in the
//! Security tab. With it on, the password step of signing in returns a short-lived challenge instead
//! of a session, and a code completes it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{Accounts, AuthedUser, Client, MeResponse, internal, password, session, totp};
use crate::{db::crypto::decode_hex, error::ApiError};

/// How long the code step may follow the password step.
const CHALLENGE_LIFETIME: Duration = Duration::from_secs(5 * 60);
/// The secret's place in the cipher's associated data, so it can't be copied to another account.
const CIPHER_SCOPE: (&str, &str) = ("2fa", "totp");
const BAD_CODE: ApiError = ApiError::Unauthorized("that code didn't work; check your authenticator app and try again");
const EXPIRED: ApiError = ApiError::Unauthorized("this sign-in has expired; enter your password again");

/// `GET /api/auth/2fa`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorStatus {
    pub enabled: bool,
    #[ts(type = "number")]
    pub recovery_codes_left: i64,
}

/// `POST /api/auth/2fa/setup`: confirm the password before a new secret is made.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorSetupRequest {
    pub password: String,
}

/// The new secret, to add to an authenticator app. Not active until confirmed with a code.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorSetup {
    /// Base32, for typing into the app by hand.
    pub secret: String,
    /// `otpauth://…`, for a QR code.
    pub otpauth_url: String,
}

/// `POST /api/auth/2fa/enable`: a code from the app, proving it was set up.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorCode {
    pub code: String,
}

/// `POST /api/auth/2fa/disable` and `/2fa/recovery-codes`: the password and a current code (or a
/// recovery code), so a stolen session alone can't do either.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorConfirm {
    pub password: String,
    pub code: String,
}

/// Recovery codes, shown once.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryCodes {
    pub codes: Vec<String>,
}

/// What signing in returns instead of a session when the account has two-factor on.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorChallenge {
    /// Send back with the code to `POST /api/auth/login/2fa`. Works for 5 minutes and 5 tries.
    pub two_factor_challenge: String,
}

/// `POST /api/auth/login/2fa`.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TwoFactorLogin {
    pub challenge: String,
    /// A 6-digit code from the app, or a recovery code.
    pub code: String,
}

impl Accounts {
    pub async fn two_factor_status(&self, user: &AuthedUser) -> Result<TwoFactorStatus, ApiError> {
        let row = self.db.two_factor(user.id).await.map_err(internal)?;
        Ok(TwoFactorStatus { enabled: row.enabled, recovery_codes_left: row.recovery_codes_left })
    }

    /// The password step passed and the account has two-factor on: a challenge for the code step.
    pub(super) async fn two_factor_challenge(&self, user: Uuid) -> Result<String, ApiError> {
        let token = session::new_token();
        self.db.create_login_challenge(&session::hash(&token), user, CHALLENGE_LIFETIME).await.map_err(internal)?;
        Ok(token)
    }

    /// The code step of signing in. Wrong codes count against the challenge (5 tries) and the IP.
    pub async fn complete_two_factor(
        &self,
        req: TwoFactorLogin,
        client: Client<'_>,
    ) -> Result<(String, MeResponse), ApiError> {
        if let Some(ip) = client.ip {
            self.by_ip.check(ip).map_err(ApiError::TooManyRequests)?;
        }
        let challenge = req.challenge.trim();
        if challenge.len() != 64 || !challenge.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(EXPIRED);
        }
        let hash = session::hash(&challenge.to_ascii_lowercase());
        let user = self.db.attempt_login_challenge(&hash).await.map_err(internal)?.ok_or(EXPIRED)?;
        if !self.check_code(user, &req.code).await? {
            return Err(BAD_CODE);
        }
        self.db.delete_login_challenge(&hash).await.map_err(internal)?;
        let (email, email_verified) = self.db.user_by_id(user).await.map_err(internal)?.ok_or(EXPIRED)?;
        self.db.ensure_user_workspace(user, None).await.map_err(internal)?;
        self.start_session(AuthedUser { id: user, email, email_verified }, client).await
    }

    /// A new, unconfirmed secret. Needs the password; refused while two-factor is on (turn it off
    /// first), so a stolen session can't move it to another phone.
    pub async fn two_factor_setup(
        &self,
        user: &AuthedUser,
        req: TwoFactorSetupRequest,
    ) -> Result<TwoFactorSetup, ApiError> {
        self.confirm_password(user, req.password).await?;
        let secret = totp::new_secret();
        let sealed = self.seal(user.id, &secret)?;
        if !self.db.set_pending_totp(user.id, &sealed).await.map_err(internal)? {
            return Err(ApiError::BadRequest(
                "two-factor sign-in is already on; turn it off first to change it".into(),
            ));
        }
        Ok(TwoFactorSetup { secret: totp::base32(&secret), otpauth_url: totp::otpauth_url(&secret, &user.email) })
    }

    /// Turns two-factor on once a code from the app matches the pending secret. Returns recovery
    /// codes, which are shown once.
    pub async fn two_factor_enable(&self, user: &AuthedUser, req: TwoFactorCode) -> Result<RecoveryCodes, ApiError> {
        self.two_factor_attempts.check(&user.id.to_string()).map_err(ApiError::TooManyRequests)?;
        let row = self.db.two_factor(user.id).await.map_err(internal)?;
        if row.enabled {
            return Err(ApiError::BadRequest("two-factor sign-in is already on".into()));
        }
        let sealed = row.sealed_secret.ok_or_else(|| ApiError::BadRequest("start the setup first".into()))?;
        let secret = self.unseal(user.id, &sealed)?;
        let step = totp::verify(&secret, &req.code, now_secs(), None).ok_or_else(|| {
            ApiError::BadRequest("that code didn't match; check the app's clock and try the next one".into())
        })?;
        let codes = totp::new_recovery_codes();
        let hashes: Vec<Vec<u8>> = codes.iter().map(|c| totp::hash_recovery_code(c)).collect();
        if !self.db.enable_totp(user.id, step, &hashes).await.map_err(internal)? {
            return Err(ApiError::BadRequest("start the setup again".into()));
        }
        tracing::info!("two-factor sign-in turned on for {}", user.email);
        Ok(RecoveryCodes { codes })
    }

    /// Turns two-factor off. Needs the password and a code (or a recovery code).
    pub async fn two_factor_disable(&self, user: &AuthedUser, req: TwoFactorConfirm) -> Result<(), ApiError> {
        self.confirm_with_code(user, req).await?;
        self.db.disable_totp(user.id).await.map_err(internal)?;
        tracing::info!("two-factor sign-in turned off for {}", user.email);
        Ok(())
    }

    /// New recovery codes; the old ones stop working. Needs the password and a code.
    pub async fn regenerate_recovery_codes(
        &self,
        user: &AuthedUser,
        req: TwoFactorConfirm,
    ) -> Result<RecoveryCodes, ApiError> {
        self.confirm_with_code(user, req).await?;
        let codes = totp::new_recovery_codes();
        let hashes: Vec<Vec<u8>> = codes.iter().map(|c| totp::hash_recovery_code(c)).collect();
        self.db.replace_recovery_codes(user.id, &hashes).await.map_err(internal)?;
        Ok(RecoveryCodes { codes })
    }

    async fn confirm_with_code(&self, user: &AuthedUser, req: TwoFactorConfirm) -> Result<(), ApiError> {
        self.confirm_password(user, req.password).await?;
        if !self.db.two_factor(user.id).await.map_err(internal)?.enabled {
            return Err(ApiError::BadRequest("two-factor sign-in isn't on".into()));
        }
        if !self.check_code(user.id, &req.code).await? {
            // A 400, not a 401: the UI treats 401 as "your session ended".
            return Err(ApiError::BadRequest("that code didn't work".into()));
        }
        Ok(())
    }

    /// Checks the current password, with the per-account limit password changes use.
    async fn confirm_password(&self, user: &AuthedUser, given: String) -> Result<(), ApiError> {
        self.password_changes.check(&user.id.to_string()).map_err(ApiError::TooManyRequests)?;
        let stored = self.db.password_hash(user.id).await.map_err(internal)?;
        if password::verify(given, stored).await {
            Ok(())
        } else {
            Err(ApiError::BadRequest("current password is incorrect".into()))
        }
    }

    /// Whether `code` is a current authenticator code (not used before) or an unused recovery code
    /// for `user`, using it up if so. Limited per account, whatever the challenge or session.
    async fn check_code(&self, user: Uuid, code: &str) -> Result<bool, ApiError> {
        self.two_factor_attempts.check(&user.to_string()).map_err(ApiError::TooManyRequests)?;
        if totp::looks_like_recovery_code(code) {
            let used = self.db.use_recovery_code(user, &totp::hash_recovery_code(code)).await.map_err(internal)?;
            if used {
                tracing::info!("a recovery code was used for user {user}");
            }
            return Ok(used);
        }
        let row = self.db.two_factor(user).await.map_err(internal)?;
        let (Some(sealed), true) = (row.sealed_secret, row.enabled) else { return Ok(false) };
        let secret = self.unseal(user, &sealed)?;
        match totp::verify(&secret, code, now_secs(), row.last_step) {
            Some(step) => self.db.use_totp_step(user, step).await.map_err(internal),
            None => Ok(false),
        }
    }

    fn seal(&self, user: Uuid, secret: &[u8]) -> Result<Vec<u8>, ApiError> {
        let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        self.db.cipher.encrypt(user, CIPHER_SCOPE.0, CIPHER_SCOPE.1, &hex).map_err(internal)
    }

    fn unseal(&self, user: Uuid, sealed: &[u8]) -> Result<Vec<u8>, ApiError> {
        let hex = self.db.cipher.decrypt(user, CIPHER_SCOPE.0, CIPHER_SCOPE.1, sealed).map_err(internal)?;
        decode_hex(&hex).map_err(internal)
    }

    /// Turns two-factor off for an account from the CLI (`engine user reset-2fa`), for someone who
    /// lost both their authenticator and their recovery codes.
    pub async fn reset_two_factor(&self, email: &str) -> anyhow::Result<()> {
        let email = super::normalize_email(email).map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(self.db.reset_two_factor_by_email(&email).await?, "no account with {email}");
        Ok(())
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
