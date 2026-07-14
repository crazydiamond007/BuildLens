use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{header, request::Parts},
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{auth::Principal, error::AppError};

#[derive(Clone, Debug, Default)]
pub struct AuditContext {
    pub ip_address: Option<IpAddr>,
    pub user_agent: Option<String>,
}

impl<S> FromRequestParts<S> for AuditContext
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip_address = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect| connect.0.ip());
        let user_agent = parts
            .headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        Ok(Self {
            ip_address,
            user_agent,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn write(
    transaction: &mut Transaction<'_, Postgres>,
    principal: &Principal,
    context: &AuditContext,
    organization_id: Option<Uuid>,
    action: &str,
    entity_type: Option<&str>,
    entity_id: Option<Uuid>,
    metadata: Value,
) -> Result<(), AppError> {
    let actor_type = match principal.credential {
        crate::auth::CredentialKind::Session => "user",
        crate::auth::CredentialKind::ApiToken => "api_token",
    };
    write_actor(
        transaction,
        actor_type,
        Some(principal.user_id),
        principal.api_token_id,
        context,
        organization_id,
        action,
        entity_type,
        entity_id,
        metadata,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn write_actor(
    transaction: &mut Transaction<'_, Postgres>,
    actor_type: &str,
    actor_user_id: Option<Uuid>,
    api_token_id: Option<Uuid>,
    context: &AuditContext,
    organization_id: Option<Uuid>,
    action: &str,
    entity_type: Option<&str>,
    entity_id: Option<Uuid>,
    metadata: Value,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_logs
            (organization_id, actor_type, actor_user_id, api_token_id, action,
             entity_type, entity_id, metadata, ip_address, user_agent)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet, $10)",
    )
    .bind(organization_id)
    .bind(actor_type)
    .bind(actor_user_id)
    .bind(api_token_id)
    .bind(action)
    .bind(entity_type)
    .bind(entity_id)
    .bind(metadata)
    .bind(context.ip_address.map(|ip| ip.to_string()))
    .bind(&context.user_agent)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
