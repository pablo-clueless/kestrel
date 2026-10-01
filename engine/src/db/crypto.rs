//! Secret values are encrypted at rest with AES-256-GCM under `KESTREL_SECRETS_KEY`. The database
//! is a shared server now, so a dump or a backup alone doesn't reveal anyone's tokens.
//!
//! Stored as `nonce (12 bytes) ‖ ciphertext ‖ tag (16 bytes)`, with a random nonce per write. The
//! associated data is `<workspace id>/<environment>/<key>`, so a value copied into another row, or
//! another workspace, fails to decrypt instead of quietly showing up there.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Nonce, Payload},
};
use anyhow::Context;
use uuid::Uuid;

const NONCE_LEN: usize = 12;

pub struct SecretsCipher(Aes256Gcm);

impl SecretsCipher {
    pub const ENV_VAR: &str = "KESTREL_SECRETS_KEY";

    /// From `KESTREL_SECRETS_KEY`: 32 bytes as 64 hex characters (`openssl rand -hex 32`).
    pub fn from_env() -> anyhow::Result<Self> {
        let hex = std::env::var(Self::ENV_VAR).unwrap_or_default();
        if hex.trim().is_empty() {
            anyhow::bail!(
                "{} is not set. Secrets are encrypted with it; generate one with `openssl rand -hex 32` and keep \
                 it safe: losing it means losing every stored secret (workspaces themselves are unaffected).",
                Self::ENV_VAR
            );
        }
        Self::from_hex(hex.trim()).with_context(|| format!("{} must be 64 hex characters", Self::ENV_VAR))
    }

    pub fn from_hex(hex: &str) -> anyhow::Result<Self> {
        let key = decode_hex(hex)?;
        anyhow::ensure!(key.len() == 32, "expected 32 bytes, got {}", key.len());
        Ok(Self(Aes256Gcm::new_from_slice(&key).expect("length checked")))
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self::from_hex(&"ab".repeat(32)).unwrap()
    }

    pub fn encrypt(&self, workspace: Uuid, environment: &str, key: &str, value: &str) -> anyhow::Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = rand::random();
        let aad = aad(workspace, environment, key);
        let sealed = self
            .0
            .encrypt(&Nonce::<Aes256Gcm>::from(nonce), Payload { msg: value.as_bytes(), aad: aad.as_bytes() })
            .map_err(|_| anyhow::anyhow!("encrypting secret `{key}` failed"))?;
        Ok([&nonce[..], &sealed].concat())
    }

    pub fn decrypt(&self, workspace: Uuid, environment: &str, key: &str, stored: &[u8]) -> anyhow::Result<String> {
        anyhow::ensure!(stored.len() > NONCE_LEN, "stored secret `{key}` is truncated");
        let (nonce, sealed) = stored.split_at(NONCE_LEN);
        let nonce = Nonce::<Aes256Gcm>::try_from(nonce).expect("length checked");
        let aad = aad(workspace, environment, key);
        let plain = self.0.decrypt(&nonce, Payload { msg: sealed, aad: aad.as_bytes() }).map_err(|_| {
            anyhow::anyhow!(
                "secret `{key}` in environment `{environment}` can't be decrypted: wrong {} or tampered data",
                Self::ENV_VAR
            )
        })?;
        String::from_utf8(plain).context("decrypted secret is not UTF-8")
    }
}

fn aad(workspace: Uuid, environment: &str, key: &str) -> String {
    // Names may contain `/`, so the environment is length-prefixed to keep the encoding
    // unambiguous: ("a/b", "c") and ("a", "b/c") must differ.
    format!("{workspace}/{}:{environment}/{key}", environment.len())
}

fn decode_hex(hex: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(hex.len().is_multiple_of(2), "odd number of hex characters");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).with_context(|| format!("`{}` is not hex", &hex[i..i + 2])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_binds_values_to_their_row() {
        let cipher = SecretsCipher::for_tests();
        let ws = Uuid::new_v4();
        let sealed = cipher.encrypt(ws, "local", "token", "s3cret").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"s3cret"), "no plaintext at rest");
        assert_eq!(cipher.decrypt(ws, "local", "token", &sealed).unwrap(), "s3cret");

        assert!(cipher.decrypt(Uuid::new_v4(), "local", "token", &sealed).is_err(), "other workspace");
        assert!(cipher.decrypt(ws, "prod", "token", &sealed).is_err(), "other environment");
        assert!(cipher.decrypt(ws, "local", "apiKey", &sealed).is_err(), "other key");
        assert!(cipher.decrypt(ws, "a/b", "c", &cipher.encrypt(ws, "a", "b/c", "x").unwrap()).is_err());

        let other = SecretsCipher::from_hex(&"cd".repeat(32)).unwrap();
        assert!(other.decrypt(ws, "local", "token", &sealed).is_err(), "other key");
        assert_ne!(sealed, cipher.encrypt(ws, "local", "token", "s3cret").unwrap(), "fresh nonce per write");
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(SecretsCipher::from_hex("abcd").is_err());
        assert!(SecretsCipher::from_hex(&"zz".repeat(32)).is_err());
    }
}
