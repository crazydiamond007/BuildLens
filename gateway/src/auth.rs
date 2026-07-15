use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts},
    http::{header, request::Parts},
};
use sqlx::Row;
use uuid::Uuid;

use crate::{crypto, error::AppError, sessions, state::AppState};

pub const API_TOKEN_PREFIX_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    Session,
    ApiToken,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub user_id: Uuid,
    pub credential: CredentialKind,
    pub api_token_id: Option<Uuid>,
    pub token_organization_id: Option<Uuid>,
}

impl Principal {
    pub fn require_session(&self) -> Result<(), AppError> {
        if self.credential == CredentialKind::Session {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(authorization) = parts.headers.get(header::AUTHORIZATION) {
            let value = authorization.to_str().map_err(|_| AppError::Unauthorized)?;
            let token = value
                .strip_prefix("Bearer ")
                .filter(|token| !token.is_empty())
                .ok_or(AppError::Unauthorized)?;
            return resolve_api_token(state, token).await;
        }

        let token = sessions::from_headers(&parts.headers).ok_or(AppError::Unauthorized)?;
        let user_id = sessions::resolve(state, token)
            .await?
            .ok_or(AppError::Unauthorized)?;

        let active = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND is_active AND deleted_at IS NULL)",
        )
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
        if !active {
            return Err(AppError::Unauthorized);
        }

        Ok(Self {
            user_id,
            credential: CredentialKind::Session,
            api_token_id: None,
            token_organization_id: None,
        })
    }
}

impl OptionalFromRequestParts<AppState> for Principal {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        let has_credentials = parts.headers.contains_key(header::AUTHORIZATION)
            || sessions::from_headers(&parts.headers).is_some();
        if !has_credentials {
            return Ok(None);
        }
        <Self as FromRequestParts<AppState>>::from_request_parts(parts, state)
            .await
            .map(Some)
    }
}

async fn resolve_api_token(state: &AppState, token: &str) -> Result<Principal, AppError> {
    if !token.starts_with("blq_") || token.len() < API_TOKEN_PREFIX_LEN {
        return Err(AppError::Unauthorized);
    }

    let prefix = &token[..API_TOKEN_PREFIX_LEN];
    let hash = crypto::sha256(token);
    let row = sqlx::query(
        "SELECT t.id, t.user_id, t.organization_id
         FROM api_tokens t
         JOIN users u ON u.id = t.user_id
         WHERE t.token_prefix = $1
           AND t.token_hash = $2
           AND t.revoked_at IS NULL
           AND (t.expires_at IS NULL OR t.expires_at > now())
           AND 'read' = ANY(t.scopes)
           AND (
               t.organization_id IS NULL OR EXISTS (
                   SELECT 1
                   FROM organization_members m
                   JOIN organizations o ON o.id = m.organization_id
                   WHERE m.organization_id = t.organization_id
                     AND m.user_id = t.user_id
                     AND o.deleted_at IS NULL
               )
           )
           AND u.is_active
           AND u.deleted_at IS NULL",
    )
    .bind(prefix)
    .bind(hash)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let token_id: Uuid = row.try_get("id")?;
    sqlx::query(
        "UPDATE api_tokens SET last_used_at = now()
         WHERE id = $1
           AND (last_used_at IS NULL OR last_used_at < now() - interval '5 minutes')",
    )
    .bind(token_id)
    .execute(&state.db)
    .await?;

    Ok(Principal {
        user_id: row.try_get("user_id")?,
        credential: CredentialKind::ApiToken,
        api_token_id: Some(token_id),
        token_organization_id: row.try_get("organization_id")?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrganizationRole {
    Viewer,
    Member,
    Admin,
    Owner,
}

impl OrganizationRole {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn level(self) -> u8 {
        match self {
            Self::Viewer => 0,
            Self::Member => 1,
            Self::Admin => 2,
            Self::Owner => 3,
        }
    }
}

pub async fn require_organization_role(
    state: &AppState,
    principal: &Principal,
    organization_id: Uuid,
    minimum: OrganizationRole,
) -> Result<OrganizationRole, AppError> {
    if principal
        .token_organization_id
        .is_some_and(|scope| scope != organization_id)
    {
        return Err(AppError::Forbidden);
    }

    let role = sqlx::query_scalar::<_, String>(
        "SELECT m.role
         FROM organization_members m
         JOIN organizations o ON o.id = m.organization_id
         WHERE m.organization_id = $1 AND m.user_id = $2 AND o.deleted_at IS NULL",
    )
    .bind(organization_id)
    .bind(principal.user_id)
    .fetch_optional(&state.db)
    .await?
    .and_then(|role| OrganizationRole::parse(&role))
    .ok_or(AppError::Forbidden)?;

    if role.level() < minimum.level() {
        return Err(AppError::Forbidden);
    }
    Ok(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_order_matches_authorization_semantics() {
        assert!(OrganizationRole::Owner.level() > OrganizationRole::Admin.level());
        assert!(OrganizationRole::Admin.level() > OrganizationRole::Member.level());
        assert!(OrganizationRole::Member.level() > OrganizationRole::Viewer.level());
    }
}
