-- Notifications and audit logging.

CREATE TABLE notifications (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id         uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id uuid REFERENCES organizations(id) ON DELETE CASCADE,

    kind            text        NOT NULL,
    severity        text        NOT NULL DEFAULT 'info'
                      CHECK (severity IN ('info', 'success', 'warning', 'error')),
    title           text        NOT NULL,
    body            text,
    link_url        text,

    -- Loose pointer at whatever this is about. Deliberately not a foreign key:
    -- a notification should outlive the thing it refers to, and it is written by
    -- three different services about a dozen different entity types.
    entity_type     text,
    entity_id       uuid,

    read_at         timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);

-- The badge count in the header is exactly this index.
CREATE INDEX notifications_unread_idx ON notifications (user_id, created_at DESC)
    WHERE read_at IS NULL;
CREATE INDEX notifications_user_idx   ON notifications (user_id, created_at DESC);

-- Append-only. No updated_at, no UPDATE grant to anyone (see 000010): an audit
-- log you can edit is not an audit log.
CREATE TABLE audit_logs (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id uuid REFERENCES organizations(id) ON DELETE SET NULL,

    -- Not every action has a human behind it. A repo can be marked stale by a
    -- scheduled job, or a webhook can be rejected because GitHub sent it.
    actor_type      text        NOT NULL
                      CHECK (actor_type IN ('user', 'api_token', 'system', 'github')),
    actor_user_id   uuid REFERENCES users(id) ON DELETE SET NULL,
    api_token_id    uuid REFERENCES api_tokens(id) ON DELETE SET NULL,

    -- Dotted, past-tense: repository.tracking_enabled, api_token.revoked.
    action          text        NOT NULL,
    entity_type     text,
    entity_id       uuid,
    metadata        jsonb       NOT NULL DEFAULT '{}',

    ip_address      inet,
    user_agent      text,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_logs_org_idx    ON audit_logs (organization_id, created_at DESC);
CREATE INDEX audit_logs_actor_idx  ON audit_logs (actor_user_id, created_at DESC)
    WHERE actor_user_id IS NOT NULL;
CREATE INDEX audit_logs_entity_idx ON audit_logs (entity_type, entity_id, created_at DESC);
