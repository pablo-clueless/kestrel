mod api;
mod config;
mod contract;
mod engine;
mod error;
mod extract;
mod import;
mod model;
mod platform;
mod redact;
mod stats;
mod template;

use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Looks in the working directory and its parents, so the repo-root `.env` is found.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "engine=info".into()))
        .init();

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
             `fly proxy`. There is no login.",
            config.bind
        );
    }
    let addr = SocketAddr::new(config.bind, config.port);
    // One database per browser workspace, under `workspaces/`.
    let data = config.workspace_dir.join("workspaces");
    tracing::info!("data: {}", data.display());
    let app = api::router(api::AppState::new(config, model::workspaces::Workspaces::new(data)));
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
