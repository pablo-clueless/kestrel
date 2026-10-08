mod api;
mod auth;
mod config;
mod contract;
mod db;
mod engine;
mod error;
mod export;
mod extract;
mod import;
mod model;
mod platform;
mod redact;
mod stats;
mod template;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
usage: engine                        serve the API and UI
       engine migrate --all          migrate every workspace schema (run before deploying a new version)
       engine import-sqlite [dir]    copy SQLite workspaces (<dir>/<id>/kestrel.db) into Postgres;
                                     dir defaults to $KESTREL_WORKSPACE_DIR/workspaces
       engine user add <email>       create an account (works with KESTREL_SIGNUP=closed); prints
                                     a generated password once
       engine user reset-2fa <email> turn off two-factor sign-in for an account whose owner lost both
                                     their authenticator and their recovery codes";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Looks in the working directory and its parents, so the repo-root `.env` is found.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "engine=info".into()))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => serve().await,
        ["migrate", "--all"] => migrate_all().await,
        ["import-sqlite"] => import_sqlite(None).await,
        ["import-sqlite", dir] => import_sqlite(Some(PathBuf::from(dir))).await,
        ["user", "add", email] => add_user(email).await,
        ["user", "reset-2fa", email] => reset_two_factor(email).await,
        ["help" | "-h" | "--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Connects to Postgres and migrates the shared `auth` schema.
async fn connect(config: &config::Config) -> anyhow::Result<Arc<db::Db>> {
    let cipher = db::crypto::SecretsCipher::from_env()?;
    let db = db::Db::connect(&config.database_url, config.db_max_connections, cipher).await?;
    Ok(Arc::new(db))
}

async fn migrate_all() -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    let db = connect(&config).await?;
    let (migrated, failed) = db.migrate_all().await?;
    tracing::info!("{migrated} workspace(s) at the current schema");
    for (id, err) in &failed {
        tracing::error!("workspace {id}: {err:#}");
    }
    anyhow::ensure!(failed.is_empty(), "{} workspace(s) failed to migrate", failed.len());
    Ok(())
}

async fn import_sqlite(dir: Option<PathBuf>) -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    let dir = dir.unwrap_or_else(|| config.workspace_dir.join("workspaces"));
    let db = connect(&config).await?;
    let summary = db::import_sqlite::import_dir(&db, &dir).await?;
    tracing::info!(
        "{}: imported {}, skipped {} already in Postgres, {} failed",
        dir.display(),
        summary.imported.len(),
        summary.skipped.len(),
        summary.failed.len()
    );
    for (id, err) in &summary.failed {
        tracing::error!("workspace {id}: {err:#}");
    }
    anyhow::ensure!(summary.failed.is_empty(), "{} workspace(s) failed to import", summary.failed.len());
    Ok(())
}

async fn reset_two_factor(email: &str) -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    let db = connect(&config).await?;
    auth::Accounts::new(db, &config).reset_two_factor(email).await?;
    println!("two-factor sign-in is off for {email}; they sign in with their password and can turn it on again");
    Ok(())
}

async fn add_user(email: &str) -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    let db = connect(&config).await?;
    let password = auth::Accounts::new(db, &config).add_user(email).await?;
    // Printed once and not stored anywhere else; there's no password change until A2.
    println!("created {email}\npassword: {password}");
    if !config.auth_enabled {
        eprintln!("note: KESTREL_AUTH is off on this machine, so accounts aren't used until it's set to `on`");
    }
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    platform::high_res_timer();
    let config = config::Config::from_env()?;
    if config.token_generated {
        tracing::info!(
            "KESTREL_TOKEN is not set; generated a session token. The UI served by the engine gets it \
             automatically; `pnpm dev` needs KESTREL_TOKEN in .env."
        );
    }
    // The engine is a load generator and the UI it serves carries the session token, so without
    // accounts anything that can reach it can drive it. Loopback by default; `KESTREL_BIND` is for
    // private networks.
    if !config.bind.is_loopback() && !config.auth_enabled {
        tracing::warn!(
            "listening on {} (not loopback) with KESTREL_AUTH=off: keep it private, e.g. \
             `fly deploy --no-public-ips` + `fly proxy`, or set KESTREL_AUTH=on.",
            config.bind
        );
    }
    tracing::info!(
        "accounts: {}",
        match (config.auth_enabled, config.signup_open) {
            (false, _) => "off (a browser's workspace id is its only credential)",
            (true, true) => "on, sign-up open",
            (true, false) => "on, sign-up closed (create accounts with `engine user add`)",
        }
    );
    // Built here first so a bad sender address or host stops the engine now, not at the first email.
    match &config.smtp {
        Some(smtp) => {
            auth::mail::Mailer::smtp(smtp)?;
            tracing::info!(
                "email: on, via {}:{} ({:?}); links point at {}",
                smtp.host,
                smtp.port,
                smtp.tls,
                config.public_url.as_deref().unwrap_or("the page that asked (KESTREL_PUBLIC_URL is unset)")
            );
        }
        None if config.auth_enabled => {
            tracing::info!("email: off, so no verification or password reset (set KESTREL_SMTP_HOST)")
        }
        None => {}
    }
    let changed = config.caps.changed();
    if !changed.is_empty() {
        tracing::info!("caps changed from the defaults: {}", changed.join(", "));
    }
    let addr = SocketAddr::new(config.bind, config.port);
    let db = connect(&config).await?;
    tracing::info!("connected to Postgres; one schema per workspace");
    let state = api::AppState::new(config, db);
    if state.accounts.enabled {
        state.accounts.spawn_session_sweeper();
    }
    let app = api::router(state);
    let listener = listen(addr)?;
    tracing::info!("engine listening on http://{addr} (UI at /, API at /api)");

    // With the peer address, for per-IP sign-in rate limits.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Ctrl-C, or SIGTERM on Unix. Platforms (Render, Fly, `docker stop`) stop the container with
/// SIGTERM, and as PID 1 the engine would otherwise ignore it until the SIGKILL.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Binds `addr`. An IPv6 address also accepts IPv4 on every OS: Linux does this by default, Windows
/// doesn't, and `::` should mean "any address" everywhere.
fn listen(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    // What std's `TcpListener::bind` does on Unix: without it, a restart fails with "Address already
    // in use" while the previous process's connections sit in TIME_WAIT. Not on Windows, where
    // SO_REUSEADDR lets another process take a port that is in use.
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    tokio::net::TcpListener::from_std(socket.into())
}
