use std::time::Instant;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tracing::warn;

use crate::state::AppState;

/// Liveness. "Is this process alive and able to answer?"
///
/// It deliberately checks NOTHING. Docker and Kubernetes both react to a failing
/// liveness probe by killing the container. If this checked Postgres, a
/// thirty-second database blip would restart every gateway replica at once,
/// turning a recoverable database problem into a total outage, and the restarts
/// would then stampede the database as it came back. Liveness answers "is the
/// process wedged", and the only honest way to answer that is to reply.
pub async fn live() -> impl IntoResponse {
    Json(Liveness {
        status: "ok",
        service: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness. "Should this instance receive traffic right now?"
///
/// This one DOES check the dependencies, because the consequence of failing is
/// that a load balancer stops sending us requests. That is exactly right when
/// we cannot reach Postgres. The process stays up and keeps trying; nothing gets
/// killed, nothing stampedes, and traffic returns on its own when the check
/// passes again.
///
/// Returns 503 with a per-dependency breakdown, so a failing probe tells you
/// which dependency is down without having to go read the logs.
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let (postgres, redis) = tokio::join!(check_postgres(&state), check_redis(&state));

    let healthy = postgres.is_ok() && redis.is_ok();

    let body = Readiness {
        status: if healthy { "ready" } else { "not_ready" },
        checks: Checks {
            postgres: postgres.into(),
            redis: redis.into(),
        },
    };

    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (code, Json(body))
}

async fn check_postgres(state: &AppState) -> Result<u128, String> {
    let started = Instant::now();

    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map(|_| started.elapsed().as_millis())
        .map_err(|e| {
            warn!(error = %e, "postgres readiness check failed");
            e.to_string()
        })
}

async fn check_redis(state: &AppState) -> Result<u128, String> {
    let started = Instant::now();
    // ConnectionManager is cheap to clone and internally shared; cloning is how
    // you get a usable `&mut` without holding a lock on the shared state.
    let mut conn = state.redis.clone();

    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .map(|_| started.elapsed().as_millis())
        .map_err(|e| {
            warn!(error = %e, "redis readiness check failed");
            e.to_string()
        })
}

#[derive(Serialize)]
struct Liveness {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct Readiness {
    status: &'static str,
    checks: Checks,
}

#[derive(Serialize)]
struct Checks {
    postgres: Check,
    redis: Check,
}

#[derive(Serialize)]
struct Check {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl From<Result<u128, String>> for Check {
    fn from(result: Result<u128, String>) -> Self {
        match result {
            Ok(latency_ms) => Self {
                status: "ok",
                latency_ms: Some(latency_ms),
                error: None,
            },
            Err(error) => Self {
                status: "error",
                latency_ms: None,
                error: Some(error),
            },
        }
    }
}
