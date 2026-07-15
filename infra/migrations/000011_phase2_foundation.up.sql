-- Corrections found in the Phase 1 review, plus constraints Phase 2 relies on.

-- GitHub owns pull-request state just as it owns workflow status and conclusion.
-- A new upstream value must not halt ingestion while BuildLens waits for a
-- migration.
ALTER TABLE pull_requests DROP CONSTRAINT pull_requests_state_check;

-- The same test commonly runs in several matrix jobs in one workflow attempt.
-- Keep one result per job, while still treating two job-less results as the same
-- run-level result. Postgres' NULLS NOT DISTINCT gives NULL the latter behavior.
ALTER TABLE test_results DROP CONSTRAINT test_results_run_test_key;
ALTER TABLE test_results
    ADD CONSTRAINT test_results_run_job_test_key
    UNIQUE NULLS NOT DISTINCT (workflow_run_id, workflow_job_id, test_key);

-- OAuth callbacks can race. The database, rather than timing luck in the
-- callback handler, guarantees a user has at most one live personal workspace.
CREATE UNIQUE INDEX organizations_personal_owner_key
    ON organizations (created_by)
    WHERE kind = 'personal' AND deleted_at IS NULL;

-- Helper keys for composite foreign keys below. The leading UUID is already
-- unique; these keys exist to let child tables prove their denormalised tenant
-- and parent identifiers describe the same row.
ALTER TABLE repositories
    ADD CONSTRAINT repositories_id_org_key UNIQUE (id, organization_id);
ALTER TABLE workflows
    ADD CONSTRAINT workflows_id_repo_key UNIQUE (id, repository_id);
ALTER TABLE commits
    ADD CONSTRAINT commits_id_repo_key UNIQUE (id, repository_id);
ALTER TABLE pull_requests
    ADD CONSTRAINT pull_requests_id_repo_key UNIQUE (id, repository_id);
ALTER TABLE workflow_runs
    ADD CONSTRAINT workflow_runs_id_repo_key UNIQUE (id, repository_id);
ALTER TABLE workflow_jobs
    ADD CONSTRAINT workflow_jobs_id_run_repo_key
    UNIQUE (id, workflow_run_id, repository_id);
ALTER TABLE ai_reports
    ADD CONSTRAINT ai_reports_id_repo_key UNIQUE (id, repository_id);

ALTER TABLE workflow_runs
    ADD CONSTRAINT workflow_runs_workflow_repo_fkey
        FOREIGN KEY (workflow_id, repository_id)
        REFERENCES workflows (id, repository_id),
    ADD CONSTRAINT workflow_runs_commit_repo_fkey
        FOREIGN KEY (head_commit_id, repository_id)
        REFERENCES commits (id, repository_id),
    ADD CONSTRAINT workflow_runs_pr_repo_fkey
        FOREIGN KEY (pull_request_id, repository_id)
        REFERENCES pull_requests (id, repository_id);

ALTER TABLE workflow_jobs
    ADD CONSTRAINT workflow_jobs_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id);

ALTER TABLE build_logs
    ADD CONSTRAINT build_logs_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id),
    ADD CONSTRAINT build_logs_job_run_repo_fkey
        FOREIGN KEY (workflow_job_id, workflow_run_id, repository_id)
        REFERENCES workflow_jobs (id, workflow_run_id, repository_id);

ALTER TABLE deployments
    ADD CONSTRAINT deployments_commit_repo_fkey
        FOREIGN KEY (commit_id, repository_id)
        REFERENCES commits (id, repository_id),
    ADD CONSTRAINT deployments_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id);

ALTER TABLE test_results
    ADD CONSTRAINT test_results_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id),
    ADD CONSTRAINT test_results_job_run_repo_fkey
        FOREIGN KEY (workflow_job_id, workflow_run_id, repository_id)
        REFERENCES workflow_jobs (id, workflow_run_id, repository_id);

ALTER TABLE dora_metrics
    ADD CONSTRAINT dora_metrics_repo_org_fkey
        FOREIGN KEY (repository_id, organization_id)
        REFERENCES repositories (id, organization_id);

ALTER TABLE build_scores
    ADD CONSTRAINT build_scores_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id);

ALTER TABLE ai_reports
    ADD CONSTRAINT ai_reports_run_requires_repo
        CHECK (workflow_run_id IS NULL OR repository_id IS NOT NULL),
    ADD CONSTRAINT ai_reports_repo_org_fkey
        FOREIGN KEY (repository_id, organization_id)
        REFERENCES repositories (id, organization_id),
    ADD CONSTRAINT ai_reports_run_repo_fkey
        FOREIGN KEY (workflow_run_id, repository_id)
        REFERENCES workflow_runs (id, repository_id);

ALTER TABLE ai_recommendations
    ADD CONSTRAINT ai_recommendations_report_repo_fkey
        FOREIGN KEY (ai_report_id, repository_id)
        REFERENCES ai_reports (id, repository_id);
