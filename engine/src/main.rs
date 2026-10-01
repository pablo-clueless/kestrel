mod api;
mod config;
mod contract;
mod db;
mod engine;
mod error;
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
                                     dir defaults to $KESTREL_WORKSPACE_DIR/workspaces";

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

async fn serve() -> anyhow::Result<()> {
    platform::high_res_timer();
    let config = config::Config::from_env()?;
    if config.token_generated {
        tracing::info!(
            "KESTREL_TOKEN is not set; generated a session token. The UI served by the engine gets it \
             automatically; `pnpm dev` needs KESTREL_TOKEN in .env."
        );
    }
    // The engine is a load generator and the UI it serves carries the session token, so anything
    // that can reach it can drive it. Loopback by default; `KESTREL_BIND` is for private networks.
    if !config.bind.is_loopback() {
        tracing::warn!(
            "listening on {} (not loopback): keep it private, e.g. `fly deploy --no-public-ips` + \
             `fly proxy`. There is no login yet.",
            config.bind
        );
    }
    let addr = SocketAddr::new(config.bind, config.port);
    let db = connect(&config).await?;
    tracing::info!("connected to Postgres; one schema per browser workspace");
    let app = api::router(api::AppState::new(config, model::workspaces::Workspaces::new(db)));
    let listener = listen(addr)?;
    tracing::info!("engine listening on http://{addr} (UI at /, API at /api)");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Binds `addr`. An IPv6 address also accepts IPv4 on every OS: Linux does this by default, Windows
/// doesn't, and `::` should mean "any address" everywhere.
fn listen(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    tokio::net::TcpListener::from_std(socket.into())
}
