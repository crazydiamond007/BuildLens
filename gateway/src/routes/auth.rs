use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::{
    audit::{self, AuditContext},
    crypto::random_urlsafe,
    error::AppError,
    github::{self, GitHubUser, OAuthToken},
    sessions,
    state::AppState,
};

const OAUTH_STATE_BYTES: usize = 32;

pub async fn github_login(State(state): State<AppState>) -> Result<Redirect, AppError> {
    let csrf_state = random_urlsafe(OAUTH_STATE_BYTES)?;
    sessions::store_oauth_state(&state, &csrf_state).await?;
    let url = github::authorize_url(&state, &csrf_state)?;
    Ok(Redirect::temporary(url.as_str()))
}

pub async fn github_callback(
    State(state): State<AppState>,
    context: AuditContext,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    // Everything up to the code exchange is a failure the person can see and act
    // on, so it goes back to the sign-in page rather than rendering JSON on an
    // origin that has no way back into the app. The state is still checked
    // before the reported error, so a forged callback cannot spoof one.
    let Some(csrf_state) = query.state.as_deref() else {
        return Ok(sign_in_redirect(&state, SignInError::InvalidRequest));
    };
    if !sessions::consume_oauth_state(&state, csrf_state).await? {
        return Ok(sign_in_redirect(&state, SignInError::ExpiredState));
    }
    if let Some(error) = query.error {
        // GitHub's own wording is worth keeping, but in the log rather than the
        // redirect: it is free text from upstream, and the browser follows it.
        warn!(
            error = %error,
            description = ?query.error_description,
            "GitHub authorization was refused"
        );
        return Ok(sign_in_redirect(&state, SignInError::from_github(&error)));
    }
    let Some(code) = query.code.as_deref() else {
        return Ok(sign_in_redirect(&state, SignInError::InvalidRequest));
    };
    let token = github::exchange_code(&state, code).await?;
    let github_user = github::fetch_user(&state, &token.access_token).await?;
    let user = persist_identity(&state, &context, github_user, token).await?;
    let session_token = sessions::issue(&state, user.id).await?;

    let cookie = sessions::set_cookie(
        &session_token,
        state.config.session_ttl,
        state.config.environment,
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(AppError::internal)?,
    );
    Ok((headers, Redirect::to(state.config.frontend_url.as_str())).into_response())
}

/// Idempotent, and deliberately so. A caller whose session already expired, or
/// who never had one, still gets the clearing cookie back: rejecting them would
/// strand a dead cookie in a browser that has no other way to shed it. The
/// frontend owns the navigation afterwards, so this answers with no content
/// rather than a redirect.
pub async fn logout(
    State(state): State<AppState>,
    context: AuditContext,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(token) = sessions::from_headers(&headers)
        && let Some(user_id) = sessions::delete(&state, token).await?
    {
        let mut transaction = state.db.begin().await?;
        audit::write_actor(
            &mut transaction,
            "user",
            Some(user_id),
            None,
            &context,
            None,
            "auth.logged_out",
            Some("user"),
            Some(user_id),
            json!({}),
        )
        .await?;
        transaction.commit().await?;
    }

    let cookie = sessions::clear_cookie(state.config.environment);
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(AppError::internal)?,
    );
    Ok((StatusCode::NO_CONTENT, response_headers).into_response())
}

/// The subset of sign-in failures worth telling someone about. A closed set,
/// not a passthrough of whatever arrives in the query string: the reason is
/// reflected into a URL the browser follows, and GitHub is not the only party
/// that can reach the callback.
#[derive(Clone, Copy)]
enum SignInError {
    AccessDenied,
    ExpiredState,
    InvalidRequest,
}

impl SignInError {
    fn from_github(error: &str) -> Self {
        match error {
            "access_denied" => Self::AccessDenied,
            _ => Self::InvalidRequest,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AccessDenied => "access_denied",
            Self::ExpiredState => "expired_state",
            Self::InvalidRequest => "invalid_request",
        }
    }
}

fn sign_in_redirect(state: &AppState, error: SignInError) -> Response {
    let mut url = state.config.frontend_url.clone();
    url.query_pairs_mut().append_pair("error", error.as_str());
    Redirect::to(url.as_str()).into_response()
}

async fn persist_identity(
    state: &AppState,
    context: &AuditContext,
    github_user: GitHubUser,
    token: OAuthToken,
) -> Result<UserResponse, AppError> {
    let access_token_encrypted = state.token_cipher.encrypt(&token.access_token)?;
    let refresh_token_encrypted = token
        .refresh_token
        .as_deref()
        .map(|refresh| state.token_cipher.encrypt(refresh))
        .transpose()?;

    let mut transaction = state.db.begin().await?;
    // The stable GitHub ID is also a valid Postgres advisory-lock key. This
    // makes two callback tabs for the same identity serialize before either
    // decides whether signup work is needed.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(github_user.id)
        .execute(&mut *transaction)
        .await?;

    let account_user_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM github_accounts WHERE github_user_id = $1 FOR UPDATE",
    )
    .bind(github_user.id)
    .fetch_optional(&mut *transaction)
    .await?;

    let user_id = if let Some(user_id) = account_user_id {
        user_id
    } else if let Some(user_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE email = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(&github_user.email)
    .fetch_optional(&mut *transaction)
    .await?
    {
        let already_linked = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM github_accounts WHERE user_id = $1)",
        )
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await?;
        if already_linked {
            return Err(AppError::conflict(
                "this verified email belongs to another connected GitHub identity",
            ));
        }
        user_id
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO users (email, name, avatar_url, last_login_at)
             VALUES ($1, $2, $3, now()) RETURNING id",
        )
        .bind(&github_user.email)
        .bind(&github_user.name)
        .bind(&github_user.avatar_url)
        .fetch_one(&mut *transaction)
        .await
        .map_err(map_identity_conflict)?
    };

    sqlx::query(
        "UPDATE users
         SET email = $2, name = $3, avatar_url = $4, last_login_at = now()
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(&github_user.email)
    .bind(&github_user.name)
    .bind(&github_user.avatar_url)
    .execute(&mut *transaction)
    .await
    .map_err(map_identity_conflict)?;

    sqlx::query(
        "INSERT INTO github_accounts
            (user_id, github_user_id, login, avatar_url, access_token_encrypted,
             refresh_token_encrypted, token_expires_at, scopes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (github_user_id) DO UPDATE SET
             login = EXCLUDED.login,
             avatar_url = EXCLUDED.avatar_url,
             access_token_encrypted = EXCLUDED.access_token_encrypted,
             refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
             token_expires_at = EXCLUDED.token_expires_at,
             scopes = EXCLUDED.scopes,
             connected_at = now()",
    )
    .bind(user_id)
    .bind(github_user.id)
    .bind(&github_user.login)
    .bind(&github_user.avatar_url)
    .bind(access_token_encrypted)
    .bind(refresh_token_encrypted)
    .bind(token.expires_at)
    .bind(&token.scopes)
    .execute(&mut *transaction)
    .await
    .map_err(map_identity_conflict)?;

    ensure_personal_organization(
        &mut transaction,
        user_id,
        &github_user.login,
        github_user.name.as_deref(),
    )
    .await?;

    audit::write_actor(
        &mut transaction,
        "user",
        Some(user_id),
        None,
        context,
        None,
        "auth.logged_in",
        Some("user"),
        Some(user_id),
        json!({"github_user_id": github_user.id}),
    )
    .await?;
    transaction.commit().await?;

    Ok(UserResponse {
        id: user_id,
        email: github_user.email,
        name: github_user.name,
        avatar_url: github_user.avatar_url,
        github_login: github_user.login,
    })
}

async fn ensure_personal_organization(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    login: &str,
    name: Option<&str>,
) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(
             SELECT 1 FROM organizations
             WHERE created_by = $1 AND kind = 'personal' AND deleted_at IS NULL
         )",
    )
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await?;
    if exists {
        return Ok(());
    }

    let organization_id = Uuid::now_v7();
    let slug = personal_slug(login, user_id);
    let display_name = format!("{}'s workspace", name.unwrap_or(login));
    sqlx::query(
        "INSERT INTO organizations (id, slug, name, kind, created_by)
         VALUES ($1, $2, $3, 'personal', $4)",
    )
    .bind(organization_id)
    .bind(slug)
    .bind(display_name)
    .bind(user_id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn personal_slug(login: &str, user_id: Uuid) -> String {
    let normalized: String = login
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let compact_id = user_id.simple().to_string();
    format!("{}-{}", normalized.trim_matches('-'), &compact_id[..8])
}

fn map_identity_conflict(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "23505")
    {
        AppError::conflict("this GitHub identity or email is already connected")
    } else {
        AppError::from(error)
    }
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Serialize)]
struct UserResponse {
    id: Uuid,
    email: String,
    name: Option<String>,
    avatar_url: Option<String>,
    github_login: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_slug_is_stable_and_url_safe() {
        let user_id = Uuid::parse_str("018f47c0-4e8a-7f00-8000-000000000001").unwrap();
        assert_eq!(personal_slug("Jane.Doe", user_id), "jane-doe-018f47c0");
    }
}
