-- Deployments and test results are the raw facts used to compute DORA and
-- flakiness. Everything in this file is a fact observed from GitHub. Everything in
-- 000006 is a number we derived from these facts.

-- Deployments come from two places, and `source` records which.
--
-- Most repositories never call the GitHub Deployments API. They just run a
-- workflow that happens to deploy. Ingesting only real Deployment objects gives
-- clean data and empty dashboards. Inferring deployments from successful
-- default-branch workflow runs gives every repo DORA metrics at the cost of a
-- heuristic. We do both and label the difference, so the UI can be honest about
-- which repos have real deployment tracking and which are inferred.
CREATE TABLE deployments (
    id                   uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id        uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    -- NULL when inferred: there is no GitHub Deployment object behind it.
    github_deployment_id bigint UNIQUE,
    workflow_run_id      uuid REFERENCES workflow_runs(id) ON DELETE SET NULL,
    environment          text        NOT NULL,
    sha                  text        NOT NULL,
    ref                  text,
    -- Soft FK, same reason as workflow_runs.head_commit_id.
    commit_id            uuid REFERENCES commits(id) ON DELETE SET NULL,
    -- GitHub owns this vocabulary (deployment_status.state), so: free text.
    status               text        NOT NULL,
    source               text        NOT NULL
                           CHECK (source IN ('github_deployment', 'workflow_inferred')),
    -- Change failure rate and MTTR only mean anything for production. Which
    -- environments count as production is configurable per repo later; for now
    -- it is a flag set at ingest.
    is_production        boolean     NOT NULL DEFAULT false,
    creator_login        text,
    started_at           timestamptz,
    -- Set when the deployment reaches a success state. NULL means in flight or
    -- failed. The difference is in `status`.
    deployed_at          timestamptz,
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now(),

    -- A real GitHub deployment must carry its GitHub id; an inferred one must
    -- carry the run it was inferred from. Neither is optional.
    CONSTRAINT deployments_source_consistency CHECK (
        (source = 'github_deployment' AND github_deployment_id IS NOT NULL)
        OR
        (source = 'workflow_inferred' AND workflow_run_id IS NOT NULL)
    )
);

CREATE TRIGGER deployments_set_updated_at
    BEFORE UPDATE ON deployments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Idempotency for the inferred path: replaying a workflow_run event must not
-- create a second deployment. (The GitHub path is already protected by the
-- UNIQUE on github_deployment_id.)
CREATE UNIQUE INDEX deployments_inferred_key ON deployments (workflow_run_id, environment)
    WHERE source = 'workflow_inferred';

CREATE INDEX deployments_repo_env_idx ON deployments (repository_id, environment, deployed_at DESC);
-- The deployment-frequency and MTTR queries both scan production deployments
-- for one repo over a time window.
CREATE INDEX deployments_prod_idx ON deployments (repository_id, deployed_at DESC)
    WHERE is_production;

-- One row per test, per run. This and workflow_runs are the two tables that
-- will actually grow. A monorepo with 5,000 tests and 50 builds a day writes a
-- quarter of a million rows here every day. Kept deliberately narrow because of
-- that; the full failure output stays in the log object in S3.
CREATE TABLE test_results (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id   uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    workflow_run_id uuid        NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    workflow_job_id uuid REFERENCES workflow_jobs(id) ON DELETE SET NULL,
    -- The stable identity of a test across time: a canonical hash of
    -- suite + classname + name. Renaming a test starts a new history, which is
    -- the honest behaviour because a renamed test is not the same test.
    test_key        text        NOT NULL,
    suite           text,
    classname       text,
    name            text        NOT NULL,
    status          text        NOT NULL CHECK (status IN ('passed', 'failed', 'skipped', 'error')),
    duration_ms     bigint,
    failure_type    text,
    failure_message text,
    executed_at     timestamptz NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),

    -- One result per test per run. A test that retries WITHIN a single run
    -- collapses to its final outcome; retries ACROSS runs are what flakiness
    -- detection actually looks at, and those are separate rows.
    CONSTRAINT test_results_run_test_key UNIQUE (workflow_run_id, test_key)
);

-- The core flakiness scan asks for every result for one test, newest first.
CREATE INDEX test_results_history_idx ON test_results (repository_id, test_key, executed_at DESC);
-- "What broke in this build?"
CREATE INDEX test_results_failures_idx ON test_results (repository_id, executed_at DESC)
    WHERE status IN ('failed', 'error');

-- The derived flakiness verdict for a test. Written by the analytics service.
CREATE TABLE flaky_tests (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id   uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    test_key        text        NOT NULL,
    suite           text,
    classname       text,
    name            text        NOT NULL,
    window_days     integer     NOT NULL DEFAULT 30,
    total_runs      integer     NOT NULL DEFAULT 0,
    passed_runs     integer     NOT NULL DEFAULT 0,
    failed_runs     integer     NOT NULL DEFAULT 0,
    -- The number of times this test changed outcome on an UNCHANGED commit.
    -- This, rather than the raw failure rate, makes a test flaky instead of
    -- merely broken. A test that fails 100% of the time is failing, not flaky.
    flip_count      integer     NOT NULL DEFAULT 0,
    flake_rate      numeric(5, 4) NOT NULL DEFAULT 0 CHECK (flake_rate BETWEEN 0 AND 1),
    is_flaky        boolean     NOT NULL DEFAULT false,
    is_quarantined  boolean     NOT NULL DEFAULT false,
    first_seen_at   timestamptz,
    last_seen_at    timestamptz,
    last_failed_at  timestamptz,
    computed_at     timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT flaky_tests_repo_test_key UNIQUE (repository_id, test_key)
);

CREATE TRIGGER flaky_tests_set_updated_at
    BEFORE UPDATE ON flaky_tests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX flaky_tests_repo_rate_idx ON flaky_tests (repository_id, flake_rate DESC)
    WHERE is_flaky;
