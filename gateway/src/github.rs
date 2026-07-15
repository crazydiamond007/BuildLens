use chrono::{DateTime, Duration, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{error::AppError, state::AppState};

const AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_URL: &str = "https://api.github.com/user";
const EMAILS_URL: &str = "https://api.github.com/user/emails";
const OAUTH_SCOPES: &str = "read:user user:email repo read:org";

pub fn authorize_url(state: &AppState, csrf_state: &str) -> Result<Url, AppError> {
    let mut url = Url::parse(AUTHORIZE_URL).map_err(AppError::internal)?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.config.github_client_id)
        .append_pair("redirect_uri", &state.config.github_redirect_uri)
        .append_pair("scope", OAUTH_SCOPES)
        .append_pair("state", csrf_state)
        .append_pair("allow_signup", "true");
    Ok(url)
}

pub async fn exchange_code(state: &AppState, code: &str) -> Result<OAuthToken, AppError> {
    let response = state
        .http
        .post(TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&TokenRequest {
            client_id: &state.config.github_client_id,
            client_secret: &state.config.github_client_secret,
            code,
            redirect_uri: &state.config.github_redirect_uri,
        })
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;

    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "token exchange returned {}",
            response.status()
        )));
    }

    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    if let Some(error) = token.error {
        return Err(AppError::Upstream(format!(
            "OAuth error {error}: {}",
            token.error_description.unwrap_or_default()
        )));
    }
    let access_token = token
        .access_token
        .ok_or_else(|| AppError::Upstream("token response had no access token".to_string()))?;

    Ok(OAuthToken {
        access_token,
        refresh_token: token.refresh_token,
        expires_at: token
            .expires_in
            .and_then(|seconds| i64::try_from(seconds).ok())
            .map(|seconds| Utc::now() + Duration::seconds(seconds)),
        scopes: token
            .scope
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect(),
    })
}

pub async fn fetch_user(state: &AppState, access_token: &str) -> Result<GitHubUser, AppError> {
    let response = state
        .http
        .get(USER_URL)
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "user endpoint returned {}",
            response.status()
        )));
    }
    let user = response
        .json::<UserResponse>()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;

    let email = match user.email {
        Some(email) => email,
        None => fetch_primary_email(state, access_token).await?,
    };

    Ok(GitHubUser {
        id: user.id,
        login: user.login,
        name: user.name,
        avatar_url: user.avatar_url,
        email,
    })
}

async fn fetch_primary_email(state: &AppState, access_token: &str) -> Result<String, AppError> {
    let response = state
        .http
        .get(EMAILS_URL)
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "email endpoint returned {}",
            response.status()
        )));
    }

    let emails = response
        .json::<Vec<EmailResponse>>()
        .await
        .map_err(|error| AppError::Upstream(error.to_string()))?;
    emails
        .iter()
        .find(|email| email.primary && email.verified)
        .or_else(|| emails.iter().find(|email| email.verified))
        .map(|email| email.email.clone())
        .ok_or_else(|| AppError::bad_request("GitHub account must have a verified email address"))
}

pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub email: String,
}

#[derive(Serialize)]
struct TokenRequest<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct UserResponse {
    id: i64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct EmailResponse {
    email: String,
    primary: bool,
    verified: bool,
}
