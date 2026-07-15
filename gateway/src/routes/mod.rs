pub mod api_tokens;
pub mod auth;
pub mod health;
pub mod me;
pub mod organizations;
pub mod repositories;

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// The gateway's routing table.
///
/// Phase 1 has health checks and nothing else. Auth lands in Phase 2, repository
/// endpoints in Phase 3.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/auth/github/login", get(auth::github_login))
        .route("/auth/github/callback", get(auth::github_callback))
        .route("/auth/logout", get(auth::logout))
        .route("/me", get(me::get))
        .route(
            "/organizations",
            get(organizations::list).post(organizations::create),
        )
        .route(
            "/organizations/{organization_id}/members",
            get(organizations::list_members).post(organizations::add_member),
        )
        .route(
            "/organizations/{organization_id}/members/{user_id}",
            delete(organizations::remove_member),
        )
        .route(
            "/api-tokens",
            get(api_tokens::list).post(api_tokens::create),
        )
        .route("/api-tokens/{token_id}", delete(api_tokens::revoke))
        .route("/github/repositories", get(repositories::discover))
        .route(
            "/organizations/{organization_id}/repositories",
            get(repositories::list),
        )
        .route(
            "/organizations/{organization_id}/github-repositories/{github_repository_id}/tracking",
            put(repositories::enable_tracking),
        )
        .route(
            "/organizations/{organization_id}/repositories/{repository_id}/tracking",
            delete(repositories::disable_tracking),
        )
        .route("/webhooks/github", post(crate::webhooks::receive))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
