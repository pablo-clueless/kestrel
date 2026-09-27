mod api;
mod config;
mod contract;
mod engine;
mod error;
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
        tracing::warn!(
            "KESTREL_TOKEN is not set; generated a session token. The dev UI won't be able to \
             connect — copy .env.example to .env and set it there."
        );
    }

    // Loopback only: the engine is a load generator and must never be reachable from the network.
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));
    let store = model::store::WorkspaceStore::open(&config.workspace_dir)?;
    tracing::info!("workspace: {}", config.workspace_dir.join(model::store::WORKSPACE_FILE).display());
    let app = api::router(api::AppState::new(config, store));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("engine listening on http://{addr}/api");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
