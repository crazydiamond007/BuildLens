-- Derived metrics. Everything here is written by the Java analytics service and
-- read by everyone else. Nothing here is a fact from GitHub. It is all a
-- number we computed, and it can always be recomputed from 000003-000005.
--
-- That property is what lets the analytics service be a dumb, replayable
-- consumer: if we change how lead time is defined, we truncate and rebuild.

CREATE TABLE dora_metrics (
    id                     uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id        uuid        NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    -- NULL means this row is the organization-wide rollup rather than one repo.
    repository_id          uuid REFERENCES repositories(id) ON DELETE CASCADE,
    granularity            text        NOT NULL CHECK (granularity IN ('daily', 'weekly', 'monthly')),
    period_start           date        NOT NULL,
    period_end             date        NOT NULL,

    deployment_count       integer     NOT NULL DEFAULT 0,
    -- Deployments per day within the period.
    deployment_frequency   numeric(10, 4),

    -- Percentiles, not means. DORA distributions are long-tailed: one PR that
    -- sat in review over the holidays will drag a mean lead time into
    -- uselessness. p50 says what a normal change feels like; p90 says what a
    -- bad week feels like. A mean says neither.
    lead_time_p50_seconds  bigint,
    lead_time_p90_seconds  bigint,

    change_failure_rate    numeric(5, 4) CHECK (change_failure_rate BETWEEN 0 AND 1),
    failed_deployment_count integer    NOT NULL DEFAULT 0,

    mttr_p50_seconds       bigint,
    mttr_p90_seconds       bigint,

    performance_band       text CHECK (performance_band IN ('elite', 'high', 'medium', 'low')),
    -- How many data points went into the above. A change failure rate computed
    -- from two deployments is not a change failure rate, and the UI needs to be
    -- able to say so rather than drawing a confident line through noise.
    sample_size            integer     NOT NULL DEFAULT 0,

    computed_at            timestamptz NOT NULL DEFAULT now(),
    created_at             timestamptz NOT NULL DEFAULT now(),
    updated_at             timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT dora_metrics_period_order CHECK (period_end >= period_start)
);

CREATE TRIGGER dora_metrics_set_updated_at
    BEFORE UPDATE ON dora_metrics
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Two partial unique indexes rather than one nullable composite key: a NULL
-- repository_id would not collide with itself under a plain UNIQUE, so the
-- org-wide rollup would silently duplicate on every recompute.
CREATE UNIQUE INDEX dora_metrics_repo_period_key ON dora_metrics (repository_id, granularity, period_start)
    WHERE repository_id IS NOT NULL;
CREATE UNIQUE INDEX dora_metrics_org_period_key  ON dora_metrics (organization_id, granularity, period_start)
    WHERE repository_id IS NULL;

CREATE INDEX dora_metrics_lookup_idx ON dora_metrics (organization_id, granularity, period_start DESC);

-- A repository's health over a trailing window. History is retained (one row
-- per computation) so the dashboard can draw "is this getting better?", which
-- is the only version of a score anyone actually acts on.
CREATE TABLE repository_scores (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id      uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    window_days        integer     NOT NULL DEFAULT 30,

    overall_score      numeric(5, 2) NOT NULL CHECK (overall_score BETWEEN 0 AND 100),
    reliability_score  numeric(5, 2) CHECK (reliability_score  BETWEEN 0 AND 100),
    velocity_score     numeric(5, 2) CHECK (velocity_score     BETWEEN 0 AND 100),
    quality_score      numeric(5, 2) CHECK (quality_score      BETWEEN 0 AND 100),
    efficiency_score   numeric(5, 2) CHECK (efficiency_score   BETWEEN 0 AND 100),
    grade              text CHECK (grade IN ('A', 'B', 'C', 'D', 'F')),

    -- The inputs and weights behind the number above. A score nobody can
    -- interrogate is a score nobody trusts, and pinning the sub-metrics into
    -- columns this early would freeze a scoring model we have not designed yet.
    breakdown          jsonb       NOT NULL DEFAULT '{}',

    computed_at        timestamptz NOT NULL DEFAULT now(),
    created_at         timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT repository_scores_key UNIQUE (repository_id, window_days, computed_at)
);

CREATE INDEX repository_scores_latest_idx ON repository_scores (repository_id, computed_at DESC);

-- A single build's score. One row per run, overwritten on recompute.
CREATE TABLE build_scores (
    id                uuid PRIMARY KEY DEFAULT uuidv7(),
    workflow_run_id   uuid        NOT NULL UNIQUE REFERENCES workflow_runs(id) ON DELETE CASCADE,
    repository_id     uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,

    score             numeric(5, 2) NOT NULL CHECK (score BETWEEN 0 AND 100),
    duration_score    numeric(5, 2),
    reliability_score numeric(5, 2),
    flakiness_score   numeric(5, 2),
    breakdown         jsonb       NOT NULL DEFAULT '{}',

    computed_at       timestamptz NOT NULL DEFAULT now(),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER build_scores_set_updated_at
    BEFORE UPDATE ON build_scores
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX build_scores_repo_idx ON build_scores (repository_id, computed_at DESC);
