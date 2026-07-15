# Event contracts

The RabbitMQ event schemas shared by the gateway (Rust, producer) and the
analytics (Java) and AI (Python) consumers.

**Empty until Phase 4.** The directory exists now so that the contract has an
obvious home when we get there. This avoids having the event
schema getting improvised inside whichever service happens to touch it first,
after which the other two services are reverse-engineering a wire format from
someone else's struct definitions.

What lands here in Phase 4:

- The envelope every event shares (id, type, version, occurred_at, aggregate).
- One schema per event: `workflow_run.completed`, `push`, `pull_request.*`.
- The exchange/queue/binding topology, and the dead-letter policy.
- A note on versioning: consumers must tolerate unknown fields, because the
  producer will always be deployed before they are.

Consumers are at-least-once. The transactional outbox (see
`infra/migrations/000009_ingestion.up.sql`) guarantees an event is never lost,
which necessarily means it can be delivered twice, so every consumer needs an
idempotency key. That is a constraint on the contract, not an implementation
detail, which is why it is written down here.
