use std::{path::PathBuf, time::Duration};

use anyhow::Context;
use uuid::Uuid;

const DEFAULT_PORT: u16 = 7070;
const DEFAULT_UI_ORIGINS: [&str; 2] = ["http://localhost:3000", "http://127.0.0.1:3000"];

/// Engine settings, read once at startup. Nothing here can be changed through the API.
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub token: String,
    /// True when `KESTREL_TOKEN` was unset and a random one was generated.
    pub token_generated: bool,
    /// Values accepted in the `Host` header (DNS-rebinding protection).
    pub allowed_hosts: Vec<String>,
    /// Values accepted in the `Origin` header, when one is present.
    pub allowed_origins: Vec<String>,
    pub caps: Caps,
    /// Where `kestrel.json` and `kestrel.secrets.json` live. `KESTREL_WORKSPACE_DIR`, else the
    /// working directory.
    pub workspace_dir: PathBuf,
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
        config.workspace_dir = match std::env::var("KESTREL_WORKSPACE_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir.trim()),
            _ => std::env::current_dir().context("reading the working directory")?,
        };
        Ok(config)
    }

    pub fn new(port: u16, token: String, token_generated: bool, ui_origins: Vec<String>) -> Self {
        let allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        // The engine's own origin is allowed too, for single-binary mode.
        let mut allowed_origins: Vec<String> = allowed_hosts.iter().map(|h| format!("http://{h}")).collect();
        allowed_origins.extend(ui_origins);

        Self {
            port,
            token,
            token_generated,
            allowed_hosts,
            allowed_origins,
            caps: Caps::default(),
            workspace_dir: PathBuf::new(),
        }
    }
}

fn random_token() -> String {
    // Two v4 UUIDs = 244 bits from the OS CSPRNG, hex encoded.
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
