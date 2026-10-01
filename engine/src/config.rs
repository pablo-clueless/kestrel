use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    time::Duration,
};

use anyhow::Context;
use uuid::Uuid;

const DEFAULT_PORT: u16 = 7070;
const DEFAULT_UI_ORIGINS: [&str; 2] = ["http://localhost:3000", "http://127.0.0.1:3000"];

/// Engine settings, read once at startup. Nothing here can be changed through the API.
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    /// `KESTREL_BIND`. Loopback unless deliberately changed (e.g. `::` inside a private container).
    pub bind: IpAddr,
    pub token: String,
    /// True when `KESTREL_TOKEN` was unset and a random one was generated.
    pub token_generated: bool,
    /// Values accepted in the `Host` header (DNS-rebinding protection).
    pub allowed_hosts: Vec<String>,
    /// Values accepted in the `Origin` header, when one is present.
    pub allowed_origins: Vec<String>,
    pub caps: Caps,
    /// `KESTREL_WORKSPACE_DIR`, else the working directory. Only `import-sqlite` reads it now (the
    /// old per-workspace SQLite files are under `workspaces/`).
    pub workspace_dir: PathBuf,
    /// `KESTREL_DATABASE_URL`. Required by everything except the tests that build a `Config` directly.
    pub database_url: String,
    /// `KESTREL_DB_MAX_CONNECTIONS`, default 10.
    pub db_max_connections: u32,
    /// `KESTREL_AUTH=on`: every `/api` route but health and sign-in needs a session.
    pub auth_enabled: bool,
    /// `KESTREL_SIGNUP=closed` turns off sign-up from the UI (`engine user add` still works).
    pub signup_open: bool,
    /// `KESTREL_TRUSTED_PROXY=1`: believe `Fly-Client-IP` / `X-Forwarded-For` / `X-Forwarded-Proto`.
    /// Only behind a proxy that sets them; otherwise any client could claim any IP.
    pub trusted_proxy: bool,
    /// `KESTREL_SMTP_*`: how account emails (verification, password reset) are sent. `None` turns
    /// those features off.
    pub smtp: Option<SmtpConfig>,
    /// `KESTREL_PUBLIC_URL`: where the UI is, for links in emails (e.g. `https://kestrel.fly.dev`).
    /// Unset, links use the `Origin` of the request that sent the email.
    pub public_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// TLS from the first byte (usually port 465).
    Tls,
    /// Plain, then upgraded with STARTTLS (usually 587). Refuses servers that can't upgrade.
    StartTls,
    /// No encryption, for a local catcher like Mailpit. Never for a real server.
    None,
}

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password: Option<String>,
    /// The sender, e.g. `Kestrel <no-reply@example.com>`.
    pub from: String,
}

/// Leaves out the password, so a logged `Config` can't leak it.
impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("from", &self.from)
            .finish()
    }
}

impl SmtpConfig {
    /// `None` when `KESTREL_SMTP_HOST` is unset. Once it's set, `KESTREL_SMTP_FROM` is required, and
    /// a username needs a password (and the other way round), so a half-done setup fails at startup.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        let get = |var: &str| lookup(var).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let Some(host) = get("KESTREL_SMTP_HOST") else {
            let stray = ["KESTREL_SMTP_PORT", "KESTREL_SMTP_USERNAME", "KESTREL_SMTP_PASSWORD", "KESTREL_SMTP_FROM"]
                .into_iter()
                .find(|var| get(var).is_some());
            if let Some(var) = stray {
                anyhow::bail!("{var} is set but KESTREL_SMTP_HOST isn't; set it too, or remove {var}");
            }
            return Ok(None);
        };
        let port: u16 = match get("KESTREL_SMTP_PORT") {
            Some(p) => p.parse().ok().filter(|&p| p > 0).context("KESTREL_SMTP_PORT must be a port number")?,
            None => 587,
        };
        let tls = match get("KESTREL_SMTP_TLS").map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("tls") => SmtpTls::Tls,
            Some("starttls") => SmtpTls::StartTls,
            Some("none") => SmtpTls::None,
            Some(_) => anyhow::bail!("KESTREL_SMTP_TLS must be one of: tls, starttls, none"),
            None if port == 465 => SmtpTls::Tls,
            None => SmtpTls::StartTls,
        };
        let (username, password) = (get("KESTREL_SMTP_USERNAME"), get("KESTREL_SMTP_PASSWORD"));
        anyhow::ensure!(
            username.is_some() == password.is_some(),
            "set both KESTREL_SMTP_USERNAME and KESTREL_SMTP_PASSWORD, or neither"
        );
        let from = get("KESTREL_SMTP_FROM")
            .context("KESTREL_SMTP_FROM is required with KESTREL_SMTP_HOST, e.g. Kestrel <no-reply@example.com>")?;
        Ok(Some(Self { host, port, tls, username, password, from }))
    }
}

/// Limits the API can't raise (HANDOFF → Safety rails → Caps).
#[derive(Debug, Clone)]
pub struct Caps {
    pub max_duration: Duration,
    pub max_timeout: Duration,
    pub max_samples: u32,
    pub max_warmup: u32,
    pub max_rps: u32,
    /// Also the most virtual users a closed-model test may have.
    pub max_in_flight: u32,
    /// Big-O sweeps run longer than other tests, so they have their own time cap.
    pub max_sweep_duration: Duration,
    pub max_n: u32,
    pub max_points: u32,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            max_duration: Duration::from_secs(60),
            max_timeout: Duration::from_secs(60),
            max_samples: 10_000,
            max_warmup: 1_000,
            max_rps: 1_000,
            max_in_flight: 10_000,
            max_sweep_duration: Duration::from_secs(5 * 60),
            max_n: 1_000_000,
            max_points: 40,
        }
    }
}

/// The variable for each cap, and the most it may be set to. The ceilings keep a typo (an extra
/// zero) from making the engine accept something absurd, and keep durations within the `u32`
/// milliseconds that run configs use (7 days).
const CAP_VARS: &[(&str, u64)] = &[
    ("KESTREL_MAX_DURATION_S", 7 * 24 * 3600),
    ("KESTREL_MAX_TIMEOUT_S", 3600),
    ("KESTREL_MAX_SAMPLES", 10_000_000),
    ("KESTREL_MAX_WARMUP", 1_000_000),
    ("KESTREL_MAX_RPS", 1_000_000),
    ("KESTREL_MAX_IN_FLIGHT", 1_000_000),
    ("KESTREL_MAX_SWEEP_S", 7 * 24 * 3600),
    ("KESTREL_MAX_N", 100_000_000),
    ("KESTREL_MAX_POINTS", 200),
];

impl Caps {
    /// The defaults, with any `KESTREL_MAX_*` variable applied. Raised or lowered only here, by
    /// whoever runs the engine; never through the API (HANDOFF → Safety rails → Caps).
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|var| std::env::var(var).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let mut caps = Self::default();
        for &(var, ceiling) in CAP_VARS {
            let Some(raw) = lookup(var).filter(|v| !v.trim().is_empty()) else { continue };
            let value: u64 = raw
                .trim()
                .parse()
                .ok()
                .filter(|v| (1..=ceiling).contains(v))
                .with_context(|| format!("{var} must be a whole number from 1 to {ceiling}"))?;
            let n = value as u32;
            match var {
                "KESTREL_MAX_DURATION_S" => caps.max_duration = Duration::from_secs(value),
                "KESTREL_MAX_TIMEOUT_S" => caps.max_timeout = Duration::from_secs(value),
                "KESTREL_MAX_SAMPLES" => caps.max_samples = n,
                "KESTREL_MAX_WARMUP" => caps.max_warmup = n,
                "KESTREL_MAX_RPS" => caps.max_rps = n,
                "KESTREL_MAX_IN_FLIGHT" => caps.max_in_flight = n,
                "KESTREL_MAX_SWEEP_S" => caps.max_sweep_duration = Duration::from_secs(value),
                "KESTREL_MAX_N" => caps.max_n = n,
                "KESTREL_MAX_POINTS" => caps.max_points = n,
                _ => unreachable!("every CAP_VARS entry is handled"),
            }
        }
        // Big-O needs at least 3 sizes to fit a curve (runs require points ≥ 3).
        anyhow::ensure!(caps.max_points >= 3, "KESTREL_MAX_POINTS must be at least 3");
        Ok(caps)
    }

    /// The caps that differ from the defaults, for the startup log.
    pub fn changed(&self) -> Vec<String> {
        let d = Self::default();
        let mut out = Vec::new();
        let mut note = |name: &str, now: String, default: String| {
            if now != default {
                out.push(format!("{name} {now} (default {default})"));
            }
        };
        note("duration", format!("{:?}", self.max_duration), format!("{:?}", d.max_duration));
        note("timeout", format!("{:?}", self.max_timeout), format!("{:?}", d.max_timeout));
        note("samples", self.max_samples.to_string(), d.max_samples.to_string());
        note("warmup", self.max_warmup.to_string(), d.max_warmup.to_string());
        note("rps", self.max_rps.to_string(), d.max_rps.to_string());
        note("in-flight", self.max_in_flight.to_string(), d.max_in_flight.to_string());
        note("Big-O sweep", format!("{:?}", self.max_sweep_duration), format!("{:?}", d.max_sweep_duration));
        note("n", self.max_n.to_string(), d.max_n.to_string());
        note("Big-O points", self.max_points.to_string(), d.max_points.to_string());
        out
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let port = match std::env::var("KESTREL_PORT") {
            Ok(p) => p.parse().context("KESTREL_PORT must be a port number")?,
            Err(_) => DEFAULT_PORT,
        };

        let (token, token_generated) = match std::env::var("KESTREL_TOKEN") {
            Ok(t) if !t.trim().is_empty() => (t.trim().to_owned(), false),
            _ => (random_token(), true),
        };

        let ui_origins: Vec<String> = match std::env::var("KESTREL_UI_ORIGINS") {
            Ok(list) => list.split(',').map(|o| o.trim().to_owned()).filter(|o| !o.is_empty()).collect(),
            Err(_) => DEFAULT_UI_ORIGINS.iter().map(|o| (*o).to_owned()).collect(),
        };

        let mut config = Self::new(port, token, token_generated, ui_origins);
        if let Ok(bind) = std::env::var("KESTREL_BIND")
            && !bind.trim().is_empty()
        {
            config.bind = bind.trim().parse().context("KESTREL_BIND must be an IP address, e.g. 127.0.0.1 or ::")?;
        }
        // Extra `Host` values to accept, e.g. `localhost:8000` behind `fly proxy 8000:7070`, or the
        // hostname a platform serves the app on.
        if let Ok(hosts) = std::env::var("KESTREL_ALLOWED_HOSTS") {
            // Tolerate pasted URLs (`https://host/`): the `Host` header never carries a scheme.
            let hosts = hosts.split(',').map(|h| {
                let h = h.trim();
                let h = h.strip_prefix("https://").or_else(|| h.strip_prefix("http://")).unwrap_or(h);
                h.trim_end_matches('/')
            });
            for host in hosts.filter(|h| !h.is_empty()) {
                config.allow_host(host);
            }
        }
        config.workspace_dir = match std::env::var("KESTREL_WORKSPACE_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir.trim()),
            _ => std::env::current_dir().context("reading the working directory")?,
        };
        config.database_url = match std::env::var("KESTREL_DATABASE_URL") {
            Ok(url) if !url.trim().is_empty() => url.trim().to_owned(),
            _ => anyhow::bail!(
                "KESTREL_DATABASE_URL is not set. For development, run `docker compose up -d db` and use \
                 postgres://kestrel:kestrel@localhost:5433/kestrel (see .env.example)."
            ),
        };
        if let Ok(n) = std::env::var("KESTREL_DB_MAX_CONNECTIONS")
            && !n.trim().is_empty()
        {
            config.db_max_connections = n
                .trim()
                .parse()
                .ok()
                .filter(|&n| n > 0)
                .context("KESTREL_DB_MAX_CONNECTIONS must be a positive number")?;
        }
        config.auth_enabled = choice("KESTREL_AUTH", &["off", "on"], "off")? == "on";
        config.signup_open = choice("KESTREL_SIGNUP", &["open", "closed"], "open")? == "open";
        config.trusted_proxy = choice("KESTREL_TRUSTED_PROXY", &["0", "1"], "0")? == "1";
        config.caps = Caps::from_env()?;
        config.smtp = SmtpConfig::from_lookup(|var| std::env::var(var).ok())?;
        config.public_url = match std::env::var("KESTREL_PUBLIC_URL") {
            Ok(url) if !url.trim().is_empty() => {
                let url = url.trim().trim_end_matches('/');
                anyhow::ensure!(
                    url.starts_with("https://") || url.starts_with("http://"),
                    "KESTREL_PUBLIC_URL must start with https:// (or http:// for local use)"
                );
                Some(url.to_owned())
            }
            _ => None,
        };
        Ok(config)
    }

    /// Accepts `host` in the `Host` header, and its origins: the engine serves the UI on it, over
    /// plain http locally or https behind a platform's TLS proxy.
    pub fn allow_host(&mut self, host: &str) {
        self.allowed_hosts.push(host.to_owned());
        self.allowed_origins.push(format!("http://{host}"));
        self.allowed_origins.push(format!("https://{host}"));
    }

    pub fn new(port: u16, token: String, token_generated: bool, ui_origins: Vec<String>) -> Self {
        let allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        // The engine's own origin is allowed too, for single-binary mode.
        let mut allowed_origins: Vec<String> = allowed_hosts.iter().map(|h| format!("http://{h}")).collect();
        allowed_origins.extend(ui_origins);

        Self {
            port,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            token,
            token_generated,
            allowed_hosts,
            allowed_origins,
            caps: Caps::default(),
            workspace_dir: PathBuf::new(),
            database_url: String::new(),
            db_max_connections: 10,
            auth_enabled: false,
            signup_open: true,
            trusted_proxy: false,
            smtp: None,
            public_url: None,
        }
    }
}

/// `var`, which must be one of `allowed` (case-insensitive), or `default` when unset or empty. An
/// unknown value is an error rather than a silent default: `KESTREL_AUTH=yes` must not mean off.
fn choice(var: &str, allowed: &[&str], default: &str) -> anyhow::Result<String> {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => {
            let v = v.trim().to_ascii_lowercase();
            anyhow::ensure!(allowed.contains(&v.as_str()), "{var} must be one of: {}", allowed.join(", "));
            Ok(v)
        }
        _ => Ok(default.to_owned()),
    }
}

fn random_token() -> String {
    // Two v4 UUIDs = 244 bits from the OS CSPRNG, hex encoded.
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn caps(vars: &[(&str, &str)]) -> anyhow::Result<Caps> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Caps::from_lookup(|var| vars.get(var).cloned())
    }

    #[test]
    fn caps_default_and_can_be_raised_or_lowered_by_the_operator() {
        let defaults = caps(&[]).unwrap();
        assert_eq!(defaults.max_rps, 1_000);
        assert!(defaults.changed().is_empty());

        let soak =
            caps(&[("KESTREL_MAX_DURATION_S", "7200"), ("KESTREL_MAX_RPS", "50"), ("KESTREL_MAX_TIMEOUT_S", " ")])
                .unwrap();
        assert_eq!(soak.max_duration, Duration::from_secs(7200));
        assert_eq!(soak.max_rps, 50, "lowering works too, e.g. on a shared deployment");
        assert_eq!(soak.max_timeout, Duration::from_secs(60), "blank means default");
        assert_eq!(soak.changed().len(), 2, "{:?}", soak.changed());
    }

    fn smtp(vars: &[(&str, &str)]) -> anyhow::Result<Option<SmtpConfig>> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        SmtpConfig::from_lookup(|var| vars.get(var).cloned())
    }

    #[test]
    fn smtp_is_off_by_default_and_half_setups_fail() {
        assert!(smtp(&[]).unwrap().is_none());
        let full = smtp(&[
            ("KESTREL_SMTP_HOST", "smtp.example.com"),
            ("KESTREL_SMTP_USERNAME", "kestrel"),
            ("KESTREL_SMTP_PASSWORD", "hunter2hunter2"),
            ("KESTREL_SMTP_FROM", "Kestrel <no-reply@example.com>"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!((full.port, full.tls), (587, SmtpTls::StartTls));
        assert!(!format!("{full:?}").contains("hunter2"), "Debug hides the password");
        let implicit = [("KESTREL_SMTP_HOST", "h"), ("KESTREL_SMTP_PORT", "465"), ("KESTREL_SMTP_FROM", "a@b.co")];
        assert_eq!(smtp(&implicit).unwrap().unwrap().tls, SmtpTls::Tls, "465 means TLS");

        for (vars, mentions) in [
            (&[("KESTREL_SMTP_FROM", "a@b.co")][..], "KESTREL_SMTP_HOST"),
            (&[("KESTREL_SMTP_HOST", "h")][..], "KESTREL_SMTP_FROM"),
            (
                &[("KESTREL_SMTP_HOST", "h"), ("KESTREL_SMTP_FROM", "a@b.co"), ("KESTREL_SMTP_USERNAME", "u")][..],
                "PASSWORD",
            ),
            (&[("KESTREL_SMTP_HOST", "h"), ("KESTREL_SMTP_FROM", "a@b.co"), ("KESTREL_SMTP_TLS", "ssl")][..], "TLS"),
        ] {
            let err = smtp(vars).unwrap_err().to_string();
            assert!(err.contains(mentions), "{vars:?}: {err}");
        }
    }

    #[test]
    fn caps_reject_nonsense_with_the_variable_named() {
        for (var, value) in [
            ("KESTREL_MAX_RPS", "0"),
            ("KESTREL_MAX_RPS", "10k"),
            ("KESTREL_MAX_RPS", "10000000"),
            ("KESTREL_MAX_DURATION_S", "99999999"),
            ("KESTREL_MAX_POINTS", "2"),
        ] {
            let err = caps(&[(var, value)]).unwrap_err().to_string();
            assert!(err.contains(var), "{var}={value}: {err}");
        }
    }
}
