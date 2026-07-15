use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    audit::{self, AuditContext},
    auth::{API_TOKEN_PREFIX_LEN, OrganizationRole, Principal, require_organization_role},
    crypto::{new_api_token, sha256},
    error::AppError,
    state::AppState,
};

pub async fn create(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Json(request): Json<CreateToken>,
) -> Result<(StatusCode, Json<CreatedTokenResponse>), AppError> {
    principal.require_session()?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::bad_request(
            "token name must be between 1 and 100 characters",
        ));
    }
    if request
        .expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return Err(AppError::bad_request("token expiry must be in the future"));
    }
    if let Some(organization_id) = request.organization_id {
        require_organization_role(
            &state,
            &principal,
            organization_id,
            OrganizationRole::Viewer,
        )
        .await?;
    }

    let scopes = request.scopes.unwrap_or_else(|| vec!["read".to_string()]);
    if scopes.as_slice() != ["read"] {
        return Err(AppError::bad_request(
            "Phase 2 API tokens support only the read scope",
        ));
    }

    let raw_token = new_api_token()?;
    let prefix = raw_token[..API_TOKEN_PREFIX_LEN].to_string();
    let token_id = Uuid::now_v7();
    let mut transaction = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO api_tokens
            (id, user_id, organization_id, name, token_prefix, token_hash, scopes, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(token_id)
    .bind(principal.user_id)
    .bind(request.organization_id)
    .bind(name)
    .bind(&prefix)
    .bind(sha256(&raw_token))
    .bind(&scopes)
    .bind(request.expires_at)
    .execute(&mut *transaction)
    .await?;
    audit::write(
        &mut transaction,
        &principal,
        &context,
        request.organization_id,
        "api_token.created",
        Some("api_token"),
        Some(token_id),
        json!({"name": name, "prefix": prefix, "scopes": scopes}),
    )
    .await?;
    transaction.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedTokenResponse {
            id: token_id,
            name: name.to_string(),
            token_prefix: prefix,
            token: raw_token,
            organization_id: request.organization_id,
            scopes,
            expires_at: request.expires_at,
        }),
    ))
}

pub async fn list(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Vec<TokenResponse>>, AppError> {
    principal.require_session()?;
    let tokens = sqlx::query(
        "SELECT id, organization_id, name, token_prefix, scopes, last_used_at,
                expires_at, revoked_at, created_at
         FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(principal.user_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| {
        Ok(TokenResponse {
            id: row.try_get("id")?,
            organization_id: row.try_get("organization_id")?,
            name: row.try_get("name")?,
            token_prefix: row.try_get("token_prefix")?,
            scopes: row.try_get("scopes")?,
            last_used_at: row.try_get("last_used_at")?,
            expires_at: row.try_get("expires_at")?,
            revoked_at: row.try_get("revoked_at")?,
            created_at: row.try_get("created_at")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(tokens))
}

pub async fn revoke(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    principal.require_session()?;
    let mut transaction = state.db.begin().await?;
    let organization_id = sqlx::query_scalar::<_, Option<Uuid>>(
        "UPDATE api_tokens SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
         RETURNING organization_id",
    )
    .bind(token_id)
    .bind(principal.user_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    audit::write(
        &mut transaction,
        &principal,
        &context,
        organization_id,
        "api_token.revoked",
        Some("api_token"),
        Some(token_id),
        json!({}),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateToken {
    name: String,
    organization_id: Option<Uuid>,
    scopes: Option<Vec<String>>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct CreatedTokenResponse {
    id: Uuid,
    name: String,
    token_prefix: String,
    token: String,
    organization_id: Option<Uuid>,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    id: Uuid,
    organization_id: Option<Uuid>,
    name: String,
    token_prefix: String,
    scopes: Vec<String>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}
