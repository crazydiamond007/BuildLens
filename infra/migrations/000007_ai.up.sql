-- AI-generated reports and recommendations. Written by the Python worker.

CREATE TABLE ai_reports (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    repository_id   uuid REFERENCES repositories(id) ON DELETE CASCADE,
    -- NULL for reports that are not about one specific build (a weekly digest).
    workflow_run_id uuid REFERENCES workflow_runs(id) ON DELETE CASCADE,

    kind            text        NOT NULL
                      CHECK (kind IN ('build_summary', 'failure_analysis',
                                      'weekly_digest', 'repo_health')),
    status          text        NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'processing', 'completed', 'failed')),

    title           text,
    summary         text,
    content_md      text,
    -- Structured output alongside the prose: findings, cited job/step ids, the
    -- log line ranges the model was looking at. This is what makes a summary
    -- checkable instead of a paragraph you have to take on faith.
    content         jsonb       NOT NULL DEFAULT '{}',

    -- Provenance. When you change the prompt in Phase 6, every report already in
    -- the table remains attributable to the prompt that produced it. Otherwise
    -- you cannot tell whether the new prompt is better or the builds just got
    -- easier. Cost and latency are here for the same reason: an AI feature whose
    -- unit economics you cannot see is one you cannot ship.
    model           text,
    prompt_version  text,
    input_tokens    integer,
    output_tokens   integer,
    cost_usd        numeric(10, 6),
    latency_ms      integer,
    error           text,

    requested_at    timestamptz NOT NULL DEFAULT now(),
    completed_at    timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER ai_reports_set_updated_at
    BEFORE UPDATE ON ai_reports
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Idempotency: RabbitMQ redelivers, and every redelivery is a paid LLM call.
-- One report of a given kind per run, full stop.
CREATE UNIQUE INDEX ai_reports_run_kind_key ON ai_reports (workflow_run_id, kind)
    WHERE workflow_run_id IS NOT NULL;

CREATE INDEX ai_reports_repo_idx    ON ai_reports (repository_id, created_at DESC);
CREATE INDEX ai_reports_pending_idx ON ai_reports (status, requested_at)
    WHERE status IN ('pending', 'processing');

CREATE TABLE ai_recommendations (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    ai_report_id  uuid        NOT NULL REFERENCES ai_reports(id) ON DELETE CASCADE,
    repository_id uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,

    category      text        NOT NULL
                    CHECK (category IN ('performance', 'reliability', 'security',
                                        'cost', 'testing', 'maintainability')),
    severity      text        NOT NULL
                    CHECK (severity IN ('info', 'low', 'medium', 'high', 'critical')),
    title         text        NOT NULL,
    body_md       text        NOT NULL,
    -- What the model based this on: job ids, failing test keys, log excerpts.
    -- A recommendation without evidence is a guess with good grammar.
    evidence      jsonb       NOT NULL DEFAULT '{}',

    status        text        NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'acknowledged', 'dismissed', 'resolved')),
    resolved_by   uuid REFERENCES users(id) ON DELETE SET NULL,
    resolved_at   timestamptz,

    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER ai_recommendations_set_updated_at
    BEFORE UPDATE ON ai_recommendations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX ai_recommendations_report_idx ON ai_recommendations (ai_report_id);
CREATE INDEX ai_recommendations_open_idx   ON ai_recommendations (repository_id, severity, created_at DESC)
    WHERE status = 'open';
