use std::time::Duration;

use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::Environment,
    crypto::{new_session_token, sha256_hex},
    error::AppError,
    state::AppState,
};

pub const SESSION_COOKIE: &str = "buildlens_session";
const OAUTH_STATE_TTL_SECONDS: u64 = 600;

#[derive(Serialize, Deserialize)]
struct SessionRecord {
    user_id: Uuid,
}

pub async fn issue(state: &AppState, user_id: Uuid) -> Result<String, AppError> {
    let token = new_session_token()?;
    let token_hash = sha256_hex(&token);
    let session_key = session_key(&token_hash);
    let user_key = user_sessions_key(user_id);
    let value = serde_json::to_string(&SessionRecord { user_id })?;
    let ttl = state.config.session_ttl.as_secs();
    let mut connection = state.redis.clone();

    redis::pipe()
        .atomic()
        .cmd("SET")
        .arg(&session_key)
        .arg(value)
        .arg("EX")
        .arg(ttl)
        .ignore()
        .cmd("SADD")
        .arg(&user_key)
        .arg(&token_hash)
        .ignore()
        .cmd("EXPIRE")
        .arg(&user_key)
        .arg(ttl)
        .ignore()
        .query_async::<()>(&mut connection)
        .await?;

    Ok(token)
}

pub async fn resolve(state: &AppState, token: &str) -> Result<Option<Uuid>, AppError> {
    let key = session_key(&sha256_hex(token));
    let mut connection = state.redis.clone();
    let value = redis::cmd("GET")
        .arg(key)
        .query_async::<Option<String>>(&mut connection)
        .await?;

    value
        .map(|json| serde_json::from_str::<SessionRecord>(&json).map(|record| record.user_id))
        .transpose()
        .map_err(AppError::from)
}

pub async fn delete(state: &AppState, token: &str) -> Result<Option<Uuid>, AppError> {
    let token_hash = sha256_hex(token);
    let user_id = resolve(state, token).await?;
    let mut connection = state.redis.clone();

    if let Some(user_id) = user_id {
        redis::pipe()
            .atomic()
            .cmd("DEL")
            .arg(session_key(&token_hash))
            .ignore()
            .cmd("SREM")
            .arg(user_sessions_key(user_id))
            .arg(token_hash)
            .ignore()
            .query_async::<()>(&mut connection)
            .await?;
    }

    Ok(user_id)
}

pub async fn store_oauth_state(state: &AppState, value: &str) -> Result<(), AppError> {
    let mut connection = state.redis.clone();
    let stored = redis::cmd("SET")
        .arg(oauth_state_key(value))
        .arg("1")
        .arg("EX")
        .arg(OAUTH_STATE_TTL_SECONDS)
        .arg("NX")
        .query_async::<Option<String>>(&mut connection)
        .await?;

    if stored.is_none() {
        return Err(AppError::internal("OAuth state collision"));
    }
    Ok(())
}

pub async fn consume_oauth_state(state: &AppState, value: &str) -> Result<bool, AppError> {
    let mut connection = state.redis.clone();
    let consumed = redis::cmd("GETDEL")
        .arg(oauth_state_key(value))
        .query_async::<Option<String>>(&mut connection)
        .await?;
    Ok(consumed.is_some())
}

pub fn from_headers(headers: &HeaderMap) -> Option<&str> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|cookie| {
        let (name, value) = cookie.trim().split_once('=')?;
        (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
    })
}

pub fn set_cookie(token: &str, ttl: Duration, environment: Environment) -> String {
    let secure = if environment == Environment::Production {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        ttl.as_secs(),
        secure
    )
}

pub fn clear_cookie(environment: Environment) -> String {
    let secure = if environment == Environment::Production {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        secure
    )
}

fn session_key(token_hash: &str) -> String {
    format!("session:{token_hash}")
}

fn user_sessions_key(user_id: Uuid) -> String {
    format!("user_sessions:{user_id}")
}

fn oauth_state_key(state: &str) -> String {
    format!("oauth_state:{}", sha256_hex(state))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn parses_session_cookie_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; buildlens_session=secret; locale=en"),
        );

        assert_eq!(from_headers(&headers), Some("secret"));
    }

    #[test]
    fn production_cookie_is_secure() {
        let cookie = set_cookie("secret", Duration::from_secs(60), Environment::Production);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Secure"));
    }
}
