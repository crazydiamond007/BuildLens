-- Strict consumer idempotency for the paid Phase 6 side effect. The per-run
-- ai_reports unique index prevents duplicate reports, while this ledger also
-- covers no-op events and repository-level reports where workflow_run_id is
-- NULL. The thin payload is retained as audit context for the claim; recovery
-- reloads the authoritative aggregate from Postgres rather than trusting it.
CREATE TABLE ai_event_receipts (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    event_id        uuid        NOT NULL UNIQUE,
    event_type      text        NOT NULL,
    organization_id uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id   uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    aggregate_id    uuid        NOT NULL,
    ai_report_id    uuid REFERENCES ai_reports(id) ON DELETE CASCADE,
    payload         jsonb       NOT NULL,
    status          text        NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    attempts        integer     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    locked_at       timestamptz,
    processed_at    timestamptz,
    last_error      text,
    received_at     timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT ai_event_receipts_repo_org_fkey
        FOREIGN KEY (repository_id, organization_id)
        REFERENCES repositories (id, organization_id)
);

CREATE TRIGGER ai_event_receipts_set_updated_at
    BEFORE UPDATE ON ai_event_receipts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX ai_event_receipts_recovery_idx
    ON ai_event_receipts (status, locked_at, received_at)
    WHERE status IN ('pending', 'processing');

GRANT INSERT, UPDATE, DELETE ON ai_event_receipts TO buildlens_ai;
