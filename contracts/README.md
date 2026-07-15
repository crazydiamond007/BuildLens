# Event contracts

The RabbitMQ event schemas shared by the gateway (Rust, producer) and the
analytics (Java) and AI (Python) consumers. Written down here, in one place, so
the wire format is a contract rather than a struct definition reverse-engineered
from whichever service touched it first.

## The one idea

**Events are triggers, not the source of truth.** Postgres is. An event says
"this repository has a new completed run - go recompute", and carries just enough
to route it and to dedupe it. The consumer reads Postgres for the detail. This is
deliberate: it keeps the wire format small and stable while the schema underneath
it evolves, and it means a consumer that missed an event can always recover the
full picture from the database.

## The envelope

Every event is a JSON object with this envelope. Event-specific fields live under
`data`.

```json
{
  "id": "0191...-uuid-v7",
  "type": "workflow_run.completed",
  "version": 1,
  "occurred_at": "2026-07-15T12:00:00Z",
  "aggregate": { "type": "workflow_run", "id": "0191...-uuid" },
  "organization_id": "0191...-uuid",
  "repository_id": "0191...-uuid",
  "data": { }
}
```

- `id` - a UUIDv7, unique per event. It is also the AMQP `message_id`. **This is
  the idempotency key.** A consumer that has already processed an `id` must treat
  a second delivery as a no-op.
- `type` - the event type; equals the AMQP routing key today.
- `version` - the envelope schema version. Bumped only on a breaking change.
- `aggregate` - the domain object the event is about.
- `organization_id` / `repository_id` - denormalised onto every event so a
  consumer can scope work (and authorization) without a lookup.
- `occurred_at` - when the gateway recorded the fact, not when GitHub emitted it.

## Events in Phase 4

| type | aggregate | routing key | emitted when |
| ---- | --------- | ----------- | ------------ |
| `workflow_run.completed` | `workflow_run` | `workflow_run.completed` | a run first reaches `status = completed` |
| `deployment.recorded` | `deployment` | `deployment.recorded` | a successful default-branch run is inferred to be a production deployment |

Both are emitted only on the *transition* into their state, so a replayed webhook
does not re-fire them. The outbox still guarantees at-least-once delivery, so the
transition guard makes the common case exactly-once and the idempotency key covers
the rest. Example payloads: [`workflow_run.completed.json`](workflow_run.completed.json),
[`deployment.recorded.json`](deployment.recorded.json).

Reserved for later phases, so consumers should not treat the list as closed:
`push.received`, `pull_request.merged`. The envelope does not change to add them.

## Topology

- **Exchange** `buildlens.events` - `topic`, durable. The gateway declares it and
  publishes here. It declares nothing else consumer-facing.
- **Dead-letter exchange** `buildlens.events.dlx` - `topic`, durable. Declared by
  the gateway so it exists; consumers point their queues' `x-dead-letter-exchange`
  at it.
- **Queues and bindings are the consumer's responsibility.** A consumer declares
  its own durable queue and binds it to `buildlens.events` with the routing keys
  it wants. Recommended for analytics:

  ```
  queue:   analytics.workflow_runs   (durable, x-dead-letter-exchange=buildlens.events.dlx)
  binding: buildlens.events -> analytics.workflow_runs  on  workflow_run.*
  binding: buildlens.events -> analytics.workflow_runs  on  deployment.*
  ```

Messages are published `persistent` (delivery mode 2) with publisher confirms, so
a message the broker acked survives a broker restart. Consumers should ack only
after they have durably processed (or dead-lettered) the message.

## Versioning

The producer is always deployed before its consumers. Therefore:

- **Consumers MUST ignore unknown fields.** Adding a field to `data` or the
  envelope is not a breaking change and does not bump `version`.
- Removing or repurposing a field, or changing a type, **is** breaking: it bumps
  `version`, and the producer emits both versions until consumers have migrated.
- A consumer that sees a `version` it does not understand should dead-letter the
  message, not crash.

## Delivery guarantees

Delivery is **at-least-once**, by construction: the gateway writes the fact and
the outbox row in one Postgres transaction, and the relay publishes afterwards
(`infra/migrations/000009_ingestion.up.sql`). A crash between the broker ack and
the outbox update re-publishes the message on restart. Duplicates are survivable;
silent loss is not. **Every consumer needs the idempotency key.** That is a
constraint on the contract, which is why it lives here and not in someone's code.
