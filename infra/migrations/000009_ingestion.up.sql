-- The ingestion plumbing. Neither of these tables models a domain concept, and
-- both exist because distributed systems lie to you.

-- GitHub delivers webhooks AT LEAST once, out of order, and will replay them on
-- demand from the UI. This table is what makes ingestion idempotent: the
-- X-GitHub-Delivery header is unique per delivery, so a duplicate collides here
-- and gets dropped instead of double-counting a build.
--
-- It doubles as a replay log. When a consumer has a bug, the fix is to correct
-- the code and re-drive the payloads from this table, rather than to go back to
-- GitHub and hope the events are still available.
CREATE TABLE webhook_deliveries (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    -- The X-GitHub-Delivery header.
    github_delivery_id uuid        NOT NULL UNIQUE,
    -- The X-GitHub-Event header: push, workflow_run, pull_request, ...
    event_type         text        NOT NULL,
    action             text,
    github_repo_id     bigint,
    -- Soft FK: we may receive a webhook for a repo we have not synced yet.
    repository_id      uuid REFERENCES repositories(id) ON DELETE SET NULL,

    -- Recorded, not enforced by this table. The handler rejects invalid
    -- signatures, but we keep the row, because a sudden run of bad signatures
    -- is a security signal and deleting it would erase the evidence.
    signature_valid    boolean     NOT NULL,

    status             text        NOT NULL DEFAULT 'received'
                         CHECK (status IN ('received', 'processing', 'processed',
                                           'failed', 'ignored')),
    attempts           integer     NOT NULL DEFAULT 0,
    payload            jsonb       NOT NULL,
    error              text,

    received_at        timestamptz NOT NULL DEFAULT now(),
    processed_at       timestamptz
);

-- The work queue for the webhook processor.
CREATE INDEX webhook_deliveries_pending_idx ON webhook_deliveries (received_at)
    WHERE status IN ('received', 'failed');
CREATE INDEX webhook_deliveries_repo_idx    ON webhook_deliveries (repository_id, received_at DESC);

-- Transactional outbox.
--
-- The problem: the gateway must write a workflow_run to Postgres AND publish an
-- event to RabbitMQ. These are two systems, so there is no shared transaction. A
-- crash between them either loses the event (Java never computes metrics for
-- that run, and nothing ever tells you) or publishes an event for a row that was
-- rolled back.
--
-- The fix: the gateway writes the row and the outbox entry in ONE Postgres
-- transaction. Either both land or neither does. A background relay then reads
-- pending rows, publishes to RabbitMQ, and marks them published. If the relay
-- crashes mid-publish, the row stays pending and is retried. Delivery is
-- at-least-once, and consumers must be idempotent. That is the trade we are
-- making: duplicates are survivable, silent loss is not.
--
-- The relay drains it with:
--   SELECT * FROM event_outbox
--    WHERE status = 'pending' AND available_at <= now()
--    ORDER BY id
--    FOR UPDATE SKIP LOCKED
--    LIMIT $1;
-- SKIP LOCKED is what lets several gateway replicas drain it concurrently
-- without any of them blocking on, or duplicating, each other's rows.
CREATE TABLE event_outbox (
    -- uuidv7, so ORDER BY id is chronological order. The relay preserves
    -- publish order for free.
    id             uuid PRIMARY KEY DEFAULT uuidv7(),

    aggregate_type text        NOT NULL,   -- 'workflow_run', 'repository', ...
    aggregate_id   uuid,
    event_type     text        NOT NULL,   -- 'workflow_run.completed'
    exchange       text        NOT NULL DEFAULT 'buildlens.events',
    routing_key    text        NOT NULL,

    payload        jsonb       NOT NULL,
    headers        jsonb       NOT NULL DEFAULT '{}',

    status         text        NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'published', 'failed')),
    attempts       integer     NOT NULL DEFAULT 0,
    last_error     text,
    -- Retry backoff: a failed publish is pushed into the future rather than
    -- hammered. The relay's WHERE clause honours it.
    available_at   timestamptz NOT NULL DEFAULT now(),
    published_at   timestamptz,
    created_at     timestamptz NOT NULL DEFAULT now()
);

-- The relay's only query. Partial, so the index stays small: published rows fall
-- out of it, and the index does not grow with the event history.
CREATE INDEX event_outbox_unpublished_idx ON event_outbox (available_at, id)
    WHERE status = 'pending';
