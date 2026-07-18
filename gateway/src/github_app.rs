//! GitHub App authentication: the layer that lets the gateway act as the App
//! itself rather than as a signed-in user.
//!
//! Two credentials come out of here. The **App JWT** (`app_jwt`) is signed with
//! the App's private key and authenticates the App to GitHub's `/app/*`
//! endpoints - it is only good for minting installation tokens and reading
//! installation metadata. The **installation access token**
//! (`installation_token`) is what actually reads Actions logs and repository
//! data; it is scoped to exactly the repositories an installation was granted,
//! and it is what replaced the old "borrow a member's OAuth token" hack.
//!
//! Installation tokens live about an hour, so they are cached in Redis and
//! reminted on demand. Everything that touches a repository outside a user
//! request funnels through `installation_token`.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Url, header};
use serde::{Deserialize, Serialize};

use crate::{error::AppError, state::AppState};

const API_VERSION: &str = "2022-11-28";
/// Drop a cached token a minute before GitHub expires it, so a token can never
/// die between the cache read and the request that spends it.
const EXPIRY_SKEW_SECONDS: i64 = 60;

/// A signed App JWT, valid for nine minutes.
///
/// GitHub caps App JWT lifetime at ten minutes and rejects one whose `iat` is
/// even slightly in its own future, so the claim is backdated a minute to
/// absorb ordinary clock skew between here and GitHub.
pub fn app_jwt(state: &AppState) -> Result<String, AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(AppError::internal)?
        .as_secs();
    let claims = app_claims(&state.config.github_app_id, now);
    let key = EncodingKey::from_rsa_pem(state.config.github_app_private_key.as_bytes())
        .map_err(AppError::internal)?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(AppError::internal)
}

/// An installation access token for `installation_id`, cached in Redis.
///
/// On a cache miss it signs a fresh App JWT, mints a token, and caches it until
/// just before it expires. This is the single choke point for App-scoped
/// repository access; callers never mint tokens themselves.
pub async fn installation_token(
    state: &AppState,
    installation_id: i64,
) -> Result<String, AppError> {
    let key = cache_key(installation_id);
    let mut redis = state.redis.clone();
    if let Some(token) = redis::cmd("GET")
        .arg(&key)
        .query_async::<Option<String>>(&mut redis)
        .await?
    {
        return Ok(token);
    }

    let minted = mint_installation_token(state, installation_id).await?;
    let ttl = (minted.expires_at - Utc::now()).num_seconds() - EXPIRY_SKEW_SECONDS;
    // A token already inside the skew window is still returned - it is valid
    // now - but not cached, so the next caller mints a fresh one instead of
    // reading a token about to expire.
    if ttl > 0 {
        redis::cmd("SET")
            .arg(&key)
            .arg(&minted.token)
            .arg("EX")
            .arg(ttl)
            .query_async::<()>(&mut redis)
            .await?;
    }
    Ok(minted.token)
}

/// Reads an installation's metadata with the App JWT: which account it is on,
/// whether it can see all repositories or only selected ones, and whether it is
/// suspended. Called from the setup callback (and installation webhooks) to keep
/// `github_installations` current.
pub async fn fetch_installation(
    state: &AppState,
    installation_id: i64,
) -> Result<Installation, AppError> {
    let jwt = app_jwt(state)?;
    let url = app_endpoint(
        state,
        &["app", "installations", &installation_id.to_string()],
    )?;
    let response = state
        .http
        .get(url)
        .bearer_auth(jwt)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "GitHub returned {status} reading installation {installation_id}: {body}"
        )));
    }
    let body = response
        .json::<InstallationResponse>()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    Ok(Installation {
        installation_id,
        account_login: body.account.login,
        account_id: body.account.id,
        target_type: body.target_type,
        repository_selection: body.repository_selection,
        suspended: body.suspended_at.is_some(),
    })
}

async fn mint_installation_token(
    state: &AppState,
    installation_id: i64,
) -> Result<InstallationToken, AppError> {
    let jwt = app_jwt(state)?;
    let url = app_endpoint(
        state,
        &[
            "app",
            "installations",
            &installation_id.to_string(),
            "access_tokens",
        ],
    )?;
    let response = state
        .http
        .post(url)
        .bearer_auth(jwt)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "GitHub returned {status} minting an installation token: {body}"
        )));
    }
    response
        .json::<InstallationToken>()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))
}

/// Builds an absolute API URL from the configured base. Mirrors
/// `GitHubApi::endpoint`, kept here so this module does not have to construct a
/// user-scoped API client just to reach the App endpoints.
fn app_endpoint(state: &AppState, segments: &[&str]) -> Result<Url, AppError> {
    let mut url = state.config.github_api_base_url.clone();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| AppError::internal("GitHub API base URL cannot be a base"))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn app_claims(app_id: &str, now_secs: u64) -> AppClaims {
    AppClaims {
        iat: now_secs - 60,
        // 540s total span (iat..exp), comfortably inside GitHub's 600s ceiling
        // even with the 60s backdate counted in.
        exp: now_secs + 8 * 60,
        iss: app_id.to_string(),
    }
}

fn cache_key(installation_id: i64) -> String {
    format!("github:installation_token:{installation_id}")
}

/// An installation as BuildLens records it. The numeric ids are stable; the
/// login can change under a rename, which is exactly why the account is keyed
/// by id everywhere it matters.
pub struct Installation {
    pub installation_id: i64,
    pub account_login: String,
    pub account_id: i64,
    pub target_type: String,
    pub repository_selection: String,
    pub suspended: bool,
}

#[derive(Debug, Serialize)]
struct AppClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

#[derive(Debug, Deserialize)]
struct InstallationResponse {
    account: InstallationAccount,
    target_type: String,
    repository_selection: String,
    suspended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct InstallationAccount {
    login: String,
    id: i64,
}

#[derive(Debug, Deserialize)]
struct InstallationToken {
    token: String,
    expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_are_backdated_and_stay_under_githubs_ceiling() {
        let claims = app_claims("123456", 1_000_000);
        assert_eq!(claims.iss, "123456");
        // Backdated so a slightly-fast clock here does not produce a future iat.
        assert_eq!(claims.iat, 1_000_000 - 60);
        // Comfortably under GitHub's ten-minute maximum.
        assert_eq!(claims.exp, 1_000_000 + 480);
        assert!(claims.exp - claims.iat < 600);
    }
}
