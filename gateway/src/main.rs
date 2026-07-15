mod audit;
mod auth;
mod config;
mod crypto;
mod error;
mod github;
mod routes;
mod sessions;
mod state;

use std::process::ExitCode;

use config::{Config, Environment};
use state::AppState;
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> ExitCode {
    // Loads .env when running on the host (`make dev`). In a container there is
    // no .env file and the environment is already populated, so this is a no-op
    // rather than an error.
    let _ = dotenvy::dotenv();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            // Tracing is not up yet, so this goes to stderr directly. It needs
            // to be readable: a config error is the most common reason a first
            // deploy fails, and "configuration error: DATABASE_URL is not set"
            // is worth more than a stack trace.
            eprintln!("configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    init_tracing(config.environment);

    if let Err(e) = run(config).await {
        error!(error = %e, "gateway failed to start");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = config.bind_addr;
    let environment = config.environment;

    let state = AppState::connect(config).await?;
    let listener = TcpListener::bind(bind_addr).await?;

    info!(%bind_addr, %environment, "buildlens gateway listening");

    axum::serve(
        listener,
        routes::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("shutdown complete");
    Ok(())
}

/// Without this, `docker compose down` SIGTERMs the process and every in-flight
/// request dies mid-response. With it, the listener stops accepting, in-flight
/// requests finish, and then we exit. It is the difference between a rolling
/// deploy being invisible and being a burst of 502s. It matters more once
/// the gateway is draining an outbox it should not abandon halfway.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("received SIGINT, shutting down"),
        _ = terminate => info!("received SIGTERM, shutting down"),
    }
}

fn init_tracing(environment: Environment) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("buildlens_gateway=info,tower_http=info,warn"));

    let registry = tracing_subscriber::registry().with(filter);

    // Human-readable locally; JSON in production, where logs go into something
    // that indexes fields rather than greps a string.
    match environment {
        Environment::Development => registry.with(fmt::layer().pretty()).init(),
        Environment::Production => registry.with(fmt::layer().json()).init(),
    }
}
