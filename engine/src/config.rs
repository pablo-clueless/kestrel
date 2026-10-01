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
        }
    }
}

fn random_token() -> String {
    // Two v4 UUIDs = 244 bits from the OS CSPRNG, hex encoded.
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
