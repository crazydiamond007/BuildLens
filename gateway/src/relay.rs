//! The outbox relay.
//!
//! Invariant #5 made a promise: facts and their events land in one Postgres
//! transaction, and *something* publishes them to RabbitMQ afterwards. This is
//! that something. It is the only place the gateway talks to RabbitMQ.
//!
//! Delivery is at-least-once by construction. The relay claims pending rows with
//! `FOR UPDATE SKIP LOCKED` (so several replicas can drain concurrently without
//! stepping on each other), publishes each with a broker confirm, and only marks
//! a row published once the broker acks. A crash between the ack and the UPDATE
//! re-publishes the row on restart — a duplicate, which consumers are required
//! to tolerate. The alternative, marking published before the ack, would lose
//! events silently, and silent loss is the one thing the outbox exists to
//! prevent.

use std::time::Duration;

use chrono::Utc;
use lapin::{
    BasicProperties, Channel, Connection, ConnectionProperties, ExchangeKind,
    options::{BasicPublishOptions, ConfirmSelectOptions, ExchangeDeclareOptions},
    types::FieldTable,
};
use sqlx::Row;
use tokio::{sync::watch, time};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::state::AppState;

/// The dead-letter exchange consumers point their queues at. Declared here so
/// the topology exists before any consumer binds to it; the gateway itself
/// never publishes to it.
const DEAD_LETTER_EXCHANGE: &str = "buildlens.events.dlx";
const BATCH_SIZE: i64 = 50;
/// After this many failed publishes a row is parked as `failed` rather than
/// retried forever. It stays in the table as evidence and can be re-driven by
/// hand once the cause is fixed.
const MAX_ATTEMPTS: i32 = 8;

pub async fn run(state: AppState, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        match connect(&state).await {
            Ok(channel) => {
                info!("outbox relay connected to rabbitmq");
                if let Err(e) = serve(&state, &channel, &mut shutdown).await {
                    // A connection-level error drops us back to reconnecting.
                    // Individual publish failures are handled per row and do not
                    // reach here.
                    warn!(error = %e, "outbox relay connection lost, reconnecting");
                } else {
                    // serve returns Ok only on shutdown.
                    info!("outbox relay stopped");
                    return;
                }
            }
            Err(e) => {
                warn!(error = %e, "outbox relay could not reach rabbitmq, retrying");
            }
        }

        // Back off before reconnecting, but wake immediately on shutdown.
        tokio::select! {
            _ = shutdown.changed() => {}
            _ = time::sleep(Duration::from_secs(3)) => {}
        }
    }
}

async fn connect(state: &AppState) -> Result<Channel, lapin::Error> {
    let options = ConnectionProperties::default()
        .with_executor(tokio_executor_trait::Tokio::current())
        .with_reactor(tokio_reactor_trait::Tokio);
    let connection = Connection::connect(&state.config.rabbitmq_url, options).await?;
    let channel = connection.create_channel().await?;

    // Publisher confirms: basic_publish returns a future that resolves when the
    // broker has accepted (and, for durable/routable messages, persisted) the
    // message. Without this we would be fire-and-forget and could not honour the
    // "only mark published after the broker has it" rule above.
    channel
        .confirm_select(ConfirmSelectOptions::default())
        .await?;

    let durable = ExchangeDeclareOptions {
        durable: true,
        ..Default::default()
    };
    channel
        .exchange_declare(
            crate::events::EVENT_EXCHANGE,
            ExchangeKind::Topic,
            durable,
            FieldTable::default(),
        )
        .await?;
    channel
        .exchange_declare(
            DEAD_LETTER_EXCHANGE,
            ExchangeKind::Topic,
            durable,
            FieldTable::default(),
        )
        .await?;

    Ok(channel)
}

async fn serve(
    state: &AppState,
    channel: &Channel,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), lapin::Error> {
    let mut interval = time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                // Drain fully each tick: keep publishing batches until the
                // outbox is empty, so a backlog clears promptly instead of one
                // batch per second.
                loop {
                    match drain_batch(state, channel).await {
                        Ok(0) => break,
                        Ok(_) => continue,
                        Err(RelayError::Connection(e)) => return Err(e),
                        Err(RelayError::Db(e)) => {
                            // A database blip is transient; log and wait for the
                            // next tick rather than tearing down the connection.
                            error!(error = %e, "outbox relay database error");
                            break;
                        }
                    }
                }
            }
        }
    }
}

enum RelayError {
    Connection(lapin::Error),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for RelayError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

/// Claims up to `BATCH_SIZE` pending rows, publishes each, and records the
/// outcome — all inside one transaction so the rows stay locked (and invisible
/// to other replicas via SKIP LOCKED) until their fate is decided. Returns the
/// number of rows claimed.
async fn drain_batch(state: &AppState, channel: &Channel) -> Result<u64, RelayError> {
    let mut transaction = state.db.begin().await?;
    let rows = sqlx::query(
        "SELECT id, exchange, routing_key, event_type, payload, headers, attempts
         FROM event_outbox
         WHERE status = 'pending' AND available_at <= now()
         ORDER BY id
         FOR UPDATE SKIP LOCKED
         LIMIT $1",
    )
    .bind(BATCH_SIZE)
    .fetch_all(&mut *transaction)
    .await?;

    let claimed = rows.len() as u64;
    if claimed == 0 {
        transaction.commit().await?;
        return Ok(0);
    }

    for row in rows {
        let id: Uuid = row.try_get("id").map_err(RelayError::Db)?;
        let exchange: String = row.try_get("exchange").map_err(RelayError::Db)?;
        let routing_key: String = row.try_get("routing_key").map_err(RelayError::Db)?;
        let event_type: String = row.try_get("event_type").map_err(RelayError::Db)?;
        let payload: serde_json::Value = row.try_get("payload").map_err(RelayError::Db)?;
        let attempts: i32 = row.try_get("attempts").map_err(RelayError::Db)?;
        let body = serde_json::to_vec(&payload).unwrap_or_default();

        let properties = BasicProperties::default()
            .with_content_type("application/json".into())
            .with_delivery_mode(2) // persistent
            .with_message_id(id.to_string().into())
            .with_type(event_type.into())
            .with_timestamp(Utc::now().timestamp() as u64);

        let confirm = channel
            .basic_publish(
                &exchange,
                &routing_key,
                BasicPublishOptions::default(),
                &body,
                properties,
            )
            .await
            .map_err(RelayError::Connection)?
            .await
            .map_err(RelayError::Connection)?;

        if confirm.is_ack() {
            sqlx::query(
                "UPDATE event_outbox
                 SET status = 'published', published_at = now(), last_error = NULL
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        } else {
            record_failure(&mut transaction, id, attempts, "broker returned nack").await?;
        }
    }

    transaction.commit().await?;
    Ok(claimed)
}

async fn record_failure(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    attempts: i32,
    reason: &str,
) -> Result<(), sqlx::Error> {
    let next_attempts = attempts + 1;
    if next_attempts >= MAX_ATTEMPTS {
        sqlx::query(
            "UPDATE event_outbox
             SET status = 'failed', attempts = $2, last_error = $3
             WHERE id = $1",
        )
        .bind(id)
        .bind(next_attempts)
        .bind(reason)
        .execute(&mut **transaction)
        .await?;
    } else {
        // Exponential-ish backoff, capped, so a struggling broker is not
        // hammered but a healthy one is retried quickly.
        let backoff_secs = (5 * (1_i64 << next_attempts.min(6))).min(300);
        sqlx::query(
            "UPDATE event_outbox
             SET attempts = $2, last_error = $3,
                 available_at = now() + make_interval(secs => $4)
             WHERE id = $1",
        )
        .bind(id)
        .bind(next_attempts)
        .bind(reason)
        .bind(backoff_secs as f64)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}
