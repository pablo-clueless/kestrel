//! Argon2id with OWASP's parameters (m = 19 MiB, t = 2, p = 1), stored as PHC strings so the
//! parameters travel with each hash and can be raised later. Hashing is deliberately slow, so it
//! runs on the blocking pool.

use std::sync::OnceLock;

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

pub const MIN_LEN: usize = 8;
pub const MAX_LEN: usize = 128;

fn argon2() -> Argon2<'static> {
    // Pinned rather than `Argon2::default()`, so a library upgrade can't change them silently.
    let params = Params::new(Params::DEFAULT_M_COST, Params::DEFAULT_T_COST, Params::DEFAULT_P_COST, None)
        .expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Length only, counted in characters: any character is allowed, so passphrases work (NIST
/// 800-63B). The cap keeps hashing cost bounded.
pub fn check_policy(password: &str) -> Result<(), String> {
    let len = password.chars().count();
    if len < MIN_LEN {
        return Err(format!("password must be at least {MIN_LEN} characters"));
    }
    if len > MAX_LEN {
        return Err(format!("password must be at most {MAX_LEN} characters"));
    }
    Ok(())
}

pub async fn hash(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        argon2().hash_password(password.as_bytes()).map(|h| h.to_string()).map_err(|e| anyhow::anyhow!("hashing: {e}"))
    })
    .await?
}

/// Whether `password` matches `stored`. With no stored hash (unknown email), a dummy hash is
/// verified instead, so the response takes as long as for a real account and doesn't reveal which
/// emails are registered.
pub async fn verify(password: String, stored: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || {
        let known = stored.is_some();
        let phc = stored.unwrap_or_else(|| dummy_hash().to_owned());
        let Ok(parsed) = PasswordHash::new(&phc) else { return false };
        argon2().verify_password(password.as_bytes(), &parsed).is_ok() && known
    })
    .await
    .unwrap_or(false)
}

/// Whether `stored` was made with other parameters than today's, and should be replaced the next
/// time the password is known (a successful sign-in). Every hash made now starts with the same
/// `$argon2id$v=19$m=…,t=…,p=…$` prefix, so comparing prefixes is enough.
pub fn needs_rehash(stored: &str) -> bool {
    static PREFIX: OnceLock<String> = OnceLock::new();
    let prefix = PREFIX.get_or_init(|| {
        // `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`: keep everything before the salt.
        let parts: Vec<&str> = dummy_hash().split('$').collect();
        format!("{}$", parts[..parts.len() - 2].join("$"))
    });
    !stored.starts_with(prefix.as_str())
}

fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let random: [u8; 16] = rand::random();
        argon2().hash_password(&random).expect("hashing the dummy password").to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_verify_and_unknown_users_never_match() {
        let stored = hash("correct horse battery".into()).await.unwrap();
        assert!(stored.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{stored}");
        assert!(verify("correct horse battery".into(), Some(stored.clone())).await);
        assert!(!verify("wrong horse battery".into(), Some(stored)).await);
        assert!(!verify("correct horse battery".into(), None).await);
        assert!(!verify("anything".into(), Some("not a phc string".into())).await);
    }

    #[tokio::test]
    async fn spots_hashes_made_with_other_parameters() {
        assert!(!needs_rehash(&hash("correct horse battery".into()).await.unwrap()));
        let weaker = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(8192, 1, 1, None).unwrap())
            .hash_password(b"correct horse battery")
            .unwrap()
            .to_string();
        assert!(needs_rehash(&weaker), "{weaker}");
        assert!(needs_rehash("$argon2i$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"), "other algorithm");
    }

    #[test]
    fn policy_is_length_only() {
        assert!(check_policy("short").is_err());
        assert!(check_policy("eight ch").is_ok(), "spaces are fine");
        assert!(check_policy("пароль-üñí").is_ok(), "so is non-ASCII");
        assert!(check_policy(&"x".repeat(129)).is_err());
    }
}
