use std::{sync::Arc, time::Duration};

use redis::aio::ConnectionManager;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing::info;

use crate::{config::Config, crypto::TokenCipher};

/// Shared, cheaply-cloneable handles. Axum clones this per request, so
/// everything in it is already internally reference-counted. `PgPool` and
/// `ConnectionManager` both are.
///
/// `Config` is deliberately NOT in here. Nothing in Phase 1 reads config after
/// startup, and a field that exists only because a later phase might want it is
/// dead weight the compiler is right to complain about. Phase 2 adds it back
/// when the OAuth handlers need the client credentials.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: ConnectionManager,
    pub http: reqwest::Client,
    pub token_cipher: TokenCipher,
    pub config: Arc<Config>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("postgres: {0}")]
    Postgres(#[from] sqlx::Error),
    #[error("redis: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
}

impl AppState {
    pub async fn connect(config: Config) -> Result<Self, StartupError> {
        // `connect` (not `connect_lazy`) so a bad DATABASE_URL kills the process
        // at boot rather than surfacing as a 503 on the first real request. In
        // a container that means the deploy fails visibly, which is what we
        // want.
        let db = PgPoolOptions::new()
            .max_connections(config.database_max_connections)
            .acquire_timeout(config.database_connect_timeout)
            // Postgres' own default idle timeout is effectively infinite, and a
            // pool holding dead connections through a network blip is a classic
            // source of "the first request after lunch always fails".
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
            .connect(&config.database_url)
            .await?;

        info!(
            max_connections = config.database_max_connections,
            "connected to postgres"
        );

        // ConnectionManager reconnects on its own and multiplexes commands over
        // one connection, so it is a pool in every sense that matters here.
        let client = redis::Client::open(config.redis_url.as_str())?;
        let redis = ConnectionManager::new(client).await?;

        info!("connected to redis");

        let http = reqwest::Client::builder()
            .user_agent(concat!("BuildLens/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()?;
        let token_cipher = TokenCipher::new(&config.token_encryption_key);

        Ok(Self {
            db,
            redis,
            http,
            token_cipher,
            config: Arc::new(config),
        })
    }
}
