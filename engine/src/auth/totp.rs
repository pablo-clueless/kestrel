//! Two-factor sign-in codes (HANDOFF → Accounts → Two-factor): TOTP (RFC 6238) as authenticator apps
//! do it, HMAC-SHA1, 6 digits, 30-second steps, plus single-use recovery codes.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};

const STEP_SECS: u64 = 30;
const DIGITS: u32 = 6;
/// Steps either side of now that are accepted: phone clocks drift and people type slowly.
const WINDOW: i64 = 1;
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
pub const RECOVERY_CODES: usize = 10;

/// A new secret: 160 bits, the size RFC 4226 recommends for HMAC-SHA1.
pub fn new_secret() -> Vec<u8> {
    let bytes: [u8; 20] = rand::random();
    bytes.to_vec()
}

/// RFC 4648 base32 without padding, as authenticator apps expect the key.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(BASE32[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The `otpauth://` link an authenticator app reads from a QR code.
pub fn otpauth_url(secret: &[u8], account: &str) -> String {
    let label: String = format!("Kestrel:{account}")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!(
        "otpauth://totp/{label}?secret={}&issuer=Kestrel&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        base32(secret)
    )
}

/// The code for time step `step`.
fn code_at(secret: &[u8], step: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let binary = u32::from_be_bytes([hash[offset] & 0x7f, hash[offset + 1], hash[offset + 2], hash[offset + 3]]);
    binary % 10u32.pow(DIGITS)
}

/// The time step a code typed at `unix_secs` matched, if it matches one within the window and is
/// later than `last_used` (a code can't be used twice, even within its 30 seconds).
pub fn verify(secret: &[u8], code: &str, unix_secs: u64, last_used: Option<i64>) -> Option<i64> {
    let code = code.trim().replace(' ', "");
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let typed: u32 = code.parse().ok()?;
    let now = (unix_secs / STEP_SECS) as i64;
    (now - WINDOW..=now + WINDOW)
        .filter(|&step| step >= 0 && last_used.is_none_or(|last| step > last))
        // Compared in full, not short-circuiting on the first match, so timing says nothing.
        .filter(|&step| subtle_eq(code_at(secret, step as u64), typed))
        .min()
}

fn subtle_eq(a: u32, b: u32) -> bool {
    use subtle::ConstantTimeEq;
    a.ct_eq(&b).into()
}

/// Recovery codes: `xxxx-xxxx` from an alphabet without look-alikes (~41 bits each), shown once.
pub fn new_recovery_codes() -> Vec<String> {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..RECOVERY_CODES)
        .map(|_| {
            let mut code: String = (0..8).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect();
            code.insert(4, '-');
            code
        })
        .collect()
}

/// What's stored for a recovery code: its SHA-256, after the same normalising as when one is typed
/// (case and dashes don't matter). The codes are random, so a plain hash is enough.
pub fn hash_recovery_code(code: &str) -> Vec<u8> {
    let normal: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    Sha256::digest(normal.as_bytes()).to_vec()
}

/// Whether what was typed is a recovery code rather than an authenticator code: those are 6 digits,
/// recovery codes 8 characters (some are all digits, so length is what tells them apart).
pub fn looks_like_recovery_code(code: &str) -> bool {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).count() == 8
}

/// The code an authenticator app would show at `unix_secs` for a base32 secret, as setup hands it
/// out. Tests act as the app with it.
#[cfg(test)]
pub fn code_for(base32_secret: &str, unix_secs: u64) -> String {
    let (mut bytes, mut buffer, mut bits) = (Vec::new(), 0u32, 0);
    for c in base32_secret.bytes() {
        let value = BASE32.iter().position(|&b| b == c).expect("base32") as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bytes.push((buffer >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    format!("{:06}", code_at(&bytes, unix_secs / STEP_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_app_reads_the_secret_back() {
        let secret = new_secret();
        assert_eq!(code_for(&base32(&secret), 1_700_000_000), format!("{:06}", code_at(&secret, 1_700_000_000 / 30)));
    }

    /// RFC 6238 appendix B, SHA-1, truncated to 6 digits.
    #[test]
    fn matches_the_rfc_test_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in
            [(59, 287082), (1111111109, 81804), (1111111111, 50471), (1234567890, 5924), (2000000000, 279037)]
        {
            assert_eq!(code_at(secret, time / STEP_SECS), expected, "t = {time}");
        }
        assert_eq!(verify(secret, "287082", 59, None), Some(1));
        assert_eq!(verify(secret, "081804", 1111111109, None), Some(1111111109 / 30), "leading zero");
    }

    #[test]
    fn accepts_one_step_of_drift_and_never_the_same_step_twice() {
        let secret = new_secret();
        let now = 1_700_000_000;
        let code = |t: u64| format!("{:06}", code_at(&secret, t / STEP_SECS));
        let step = (now / STEP_SECS) as i64;
        assert_eq!(verify(&secret, &code(now - 30), now, None), Some(step - 1));
        assert_eq!(verify(&secret, &code(now + 30), now, None), Some(step + 1));
        assert_eq!(verify(&secret, &code(now - 90), now, None), None, "too old");
        assert_eq!(verify(&secret, &code(now), now, Some(step)), None, "already used");
        assert_eq!(verify(&secret, &code(now + 30), now, Some(step)), Some(step + 1), "a later one is fine");
        assert_eq!(verify(&secret, "12345", now, None), None);
        assert_eq!(verify(&secret, "abcdef", now, None), None);
        assert_eq!(verify(&secret, &format!(" {} ", code(now)), now, None), Some(step), "spaces are ignored");
    }

    #[test]
    fn encodes_base32_and_builds_the_app_link() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI"); // RFC 4648 test vector, without padding
        assert_eq!(base32(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        let url = otpauth_url(b"12345678901234567890", "ada@example.com");
        assert_eq!(
            url,
            "otpauth://totp/Kestrel:ada%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Kestrel&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn recovery_codes_are_distinct_and_forgiving_to_type() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODES);
        assert_eq!(codes.iter().collect::<std::collections::HashSet<_>>().len(), RECOVERY_CODES);
        let code = &codes[0];
        assert!(looks_like_recovery_code(code) && !looks_like_recovery_code("123456"));
        assert_eq!(hash_recovery_code(code), hash_recovery_code(&code.replace('-', "").to_uppercase()));
    }
}
