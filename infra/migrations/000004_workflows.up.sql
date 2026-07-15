-- GitHub Actions: workflows, runs, jobs, steps, and the logs they produce.
--
-- TWO THINGS TO UNDERSTAND BEFORE READING THIS FILE.
--
-- 1. Status and conclusion are plain text, not enums.
--    GitHub adds values to these over time. `stale`, `action_required` and
--    `startup_failure` all appeared after the API shipped. If they were enums,
--    a new value from GitHub would make ingestion throw until we deployed a
--    migration. We do not control this vocabulary, so we do not constrain it.
--    Fields we DO control (role, severity, granularity) get CHECK constraints.
--
-- 2. Foreign keys pointing outside this file are nullable, on purpose.
--    GitHub webhooks arrive out of order and at-least-once. A
--    `workflow_run.completed` event routinely lands before the `push` that
--    created its commit. So a run always stores head_sha as text, and carries a
--    NULLABLE head_commit_id that gets backfilled when (if) the commit arrives.
--    Making that FK strict would deadlock ingestion against itself in
--    production. Within a stream, such as job to run or step to job, GitHub does
--    guarantee ordering, so those FKs are strict.

CREATE TABLE workflows (
    id                 uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id      uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    github_workflow_id bigint      NOT NULL,
    name               text        NOT NULL,
    path               text        NOT NULL,
    state              text        NOT NULL DEFAULT 'active',
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now(),
    deleted_at         timestamptz,

    CONSTRAINT workflows_repo_github_id_key UNIQUE (repository_id, github_workflow_id)
);

CREATE TRIGGER workflows_set_updated_at
    BEFORE UPDATE ON workflows
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The busiest table in the system.
--
-- The unique key is (github_run_id, run_attempt), NOT github_run_id alone.
-- When you re-run a workflow, GitHub keeps the same run_id and increments
-- run_attempt. Each attempt is its own row here. Collapsing them would destroy
-- exactly the signal flaky-test detection depends on: same commit, same
-- workflow, failed, then passed on retry.
CREATE TABLE workflow_runs (
    id                    uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id         uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    -- Nullable: a run can reference a workflow file we have not synced yet.
    workflow_id           uuid REFERENCES workflows(id) ON DELETE SET NULL,
    github_run_id         bigint      NOT NULL,
    run_attempt           integer     NOT NULL DEFAULT 1,
    run_number            integer     NOT NULL,
    name                  text,
    -- push, pull_request, schedule, workflow_dispatch, release, ...
    event                 text        NOT NULL,
    -- queued | in_progress | completed | waiting | requested | pending
    status                text        NOT NULL,
    -- NULL until the run finishes. Then: success | failure | cancelled |
    -- skipped | timed_out | action_required | neutral | stale | ...
    conclusion            text,
    head_sha              text        NOT NULL,
    head_branch           text,
    -- Soft FKs, resolved lazily. See the header.
    head_commit_id        uuid REFERENCES commits(id) ON DELETE SET NULL,
    pull_request_id       uuid REFERENCES pull_requests(id) ON DELETE SET NULL,
    actor_login           text,
    triggering_actor_login text,
    -- Denormalised from repositories.default_branch at ingest time. Every DORA
    -- query filters on it, and the branch a run targeted at the time it ran is
    -- a historical fact. It should not change if the repo later renames its
    -- default branch.
    is_default_branch     boolean     NOT NULL DEFAULT false,
    -- GitHub's timeline: queued at created_at_github, picked up at
    -- run_started_at, finished at completed_at. The gap between the first two
    -- is runner contention, which is a metric people care about.
    created_at_github     timestamptz,
    run_started_at        timestamptz,
    completed_at          timestamptz,
    queued_duration_ms    bigint GENERATED ALWAYS AS (
        CASE WHEN run_started_at IS NOT NULL AND created_at_github IS NOT NULL
             THEN (extract(epoch FROM (run_started_at - created_at_github)) * 1000)::bigint
        END
    ) STORED,
    duration_ms           bigint GENERATED ALWAYS AS (
        CASE WHEN completed_at IS NOT NULL AND run_started_at IS NOT NULL
             THEN (extract(epoch FROM (completed_at - run_started_at)) * 1000)::bigint
        END
    ) STORED,
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT workflow_runs_github_run_attempt_key UNIQUE (github_run_id, run_attempt)
);

CREATE TRIGGER workflow_runs_set_updated_at
    BEFORE UPDATE ON workflow_runs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX workflow_runs_repo_created_idx ON workflow_runs (repository_id, created_at_github DESC);
CREATE INDEX workflow_runs_repo_sha_idx     ON workflow_runs (repository_id, head_sha);
CREATE INDEX workflow_runs_workflow_idx     ON workflow_runs (workflow_id, created_at_github DESC);
CREATE INDEX workflow_runs_pr_idx           ON workflow_runs (pull_request_id) WHERE pull_request_id IS NOT NULL;
-- The commit-resolution backfill scans exactly this.
CREATE INDEX workflow_runs_unresolved_commit_idx ON workflow_runs (repository_id, head_sha)
    WHERE head_commit_id IS NULL;
-- The DORA hot path: successful runs on the default branch, newest first.
CREATE INDEX workflow_runs_dora_idx ON workflow_runs (repository_id, completed_at DESC)
    WHERE is_default_branch AND conclusion = 'success';

CREATE TABLE workflow_jobs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    workflow_run_id  uuid        NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    -- Denormalised so job-level queries do not need to join through the run.
    repository_id    uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    github_job_id    bigint      NOT NULL UNIQUE,
    run_attempt      integer     NOT NULL DEFAULT 1,
    name             text        NOT NULL,
    status           text        NOT NULL,
    conclusion       text,
    runner_id        bigint,
    runner_name      text,
    runner_group_name text,
    labels           text[]      NOT NULL DEFAULT '{}',
    started_at       timestamptz,
    completed_at     timestamptz,
    duration_ms      bigint GENERATED ALWAYS AS (
        CASE WHEN completed_at IS NOT NULL AND started_at IS NOT NULL
             THEN (extract(epoch FROM (completed_at - started_at)) * 1000)::bigint
        END
    ) STORED,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER workflow_jobs_set_updated_at
    BEFORE UPDATE ON workflow_jobs
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX workflow_jobs_run_idx  ON workflow_jobs (workflow_run_id);
CREATE INDEX workflow_jobs_repo_idx ON workflow_jobs (repository_id, completed_at DESC);
-- "Which job is our slowest?" and "which job fails most?" both start here.
CREATE INDEX workflow_jobs_repo_name_idx ON workflow_jobs (repository_id, name, completed_at DESC);

CREATE TABLE workflow_steps (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    workflow_job_id uuid        NOT NULL REFERENCES workflow_jobs(id) ON DELETE CASCADE,
    number          integer     NOT NULL,
    name            text        NOT NULL,
    status          text        NOT NULL,
    conclusion      text,
    started_at      timestamptz,
    completed_at    timestamptz,
    duration_ms     bigint GENERATED ALWAYS AS (
        CASE WHEN completed_at IS NOT NULL AND started_at IS NOT NULL
             THEN (extract(epoch FROM (completed_at - started_at)) * 1000)::bigint
        END
    ) STORED,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT workflow_steps_job_number_key UNIQUE (workflow_job_id, number)
);

CREATE TRIGGER workflow_steps_set_updated_at
    BEFORE UPDATE ON workflow_steps
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Metadata only. The log text itself lives in MinIO/S3 and never enters
-- Postgres: a single verbose CI job can emit tens of megabytes, and the AI
-- worker wants to stream it, not SELECT it.
CREATE TABLE build_logs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    repository_id    uuid        NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    workflow_run_id  uuid        NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    -- NULL means this object is the whole-run archive rather than one job's log.
    workflow_job_id  uuid REFERENCES workflow_jobs(id) ON DELETE CASCADE,
    storage_bucket   text        NOT NULL,
    object_key       text        NOT NULL UNIQUE,
    size_bytes       bigint      NOT NULL,
    content_type     text        NOT NULL DEFAULT 'text/plain',
    content_encoding text,
    sha256           bytea,
    line_count       integer,
    expires_at       timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX build_logs_run_idx ON build_logs (workflow_run_id);
CREATE INDEX build_logs_job_idx ON build_logs (workflow_job_id) WHERE workflow_job_id IS NOT NULL;
