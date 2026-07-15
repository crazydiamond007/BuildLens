//! The transactional outbox writer and the event envelope.
//!
//! A fact and its event are written to Postgres in one transaction (see
//! invariant #5). The relay publishes the row afterwards. Events are *triggers*,
//! not the source of truth: the envelope carries enough to route and to dedupe,
//! and consumers read Postgres for the full detail. That keeps the wire contract
//! small and stable while the schema underneath it evolves.
//!
//! The contract these envelopes implement is documented in `contracts/`.

use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

/// The topic exchange every event is published to. Consumers bind their own
/// queues to it; the gateway only declares the exchange.
pub const EVENT_EXCHANGE: &str = "buildlens.events";

/// Bumped only on a breaking change to the envelope. Consumers must tolerate
/// unknown fields, so adding a field is not breaking and does not bump this.
pub const SCHEMA_VERSION: u32 = 1;

/// One event to be enqueued. `routing_key` is `<aggregate>.<event>` today
/// (e.g. `workflow_run.completed`); it is separate from `event_type` so the two
/// can diverge later without touching call sites.
pub struct OutboxEvent {
    pub aggregate_type: &'static str,
    pub aggregate_id: Uuid,
    pub event_type: &'static str,
    pub routing_key: String,
    pub organization_id: Uuid,
    pub repository_id: Uuid,
    pub data: Value,
}

impl OutboxEvent {
    /// Convenience for the common case where the routing key equals the event
    /// type.
    pub fn new(
        aggregate_type: &'static str,
        aggregate_id: Uuid,
        event_type: &'static str,
        organization_id: Uuid,
        repository_id: Uuid,
        data: Value,
    ) -> Self {
        Self {
            aggregate_type,
            aggregate_id,
            event_type,
            routing_key: event_type.to_string(),
            organization_id,
            repository_id,
            data,
        }
    }
}

/// Writes the outbox row inside the caller's transaction. The envelope id is a
/// UUIDv7 generated here (not left to the column default) so it can be embedded
/// in the stored payload and reused by the relay as the RabbitMQ `message_id` —
/// the same value the consumer dedupes on. Returns that id.
pub async fn enqueue(
    transaction: &mut Transaction<'_, Postgres>,
    event: OutboxEvent,
) -> Result<Uuid, AppError> {
    let id = Uuid::now_v7();
    let envelope = json!({
        "id": id,
        "type": event.event_type,
        "version": SCHEMA_VERSION,
        "occurred_at": Utc::now(),
        "aggregate": { "type": event.aggregate_type, "id": event.aggregate_id },
        "organization_id": event.organization_id,
        "repository_id": event.repository_id,
        "data": event.data,
    });
    let headers = json!({
        "event_type": event.event_type,
        "aggregate_type": event.aggregate_type,
        "schema_version": SCHEMA_VERSION,
    });

    sqlx::query(
        "INSERT INTO event_outbox
            (id, aggregate_type, aggregate_id, event_type, exchange, routing_key,
             payload, headers)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(event.aggregate_type)
    .bind(event.aggregate_id)
    .bind(event.event_type)
    .bind(EVENT_EXCHANGE)
    .bind(&event.routing_key)
    .bind(&envelope)
    .bind(&headers)
    .execute(&mut **transaction)
    .await?;

    Ok(id)
}
