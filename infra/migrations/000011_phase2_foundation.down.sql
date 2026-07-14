ALTER TABLE ai_recommendations
    DROP CONSTRAINT ai_recommendations_report_repo_fkey;

ALTER TABLE ai_reports
    DROP CONSTRAINT ai_reports_run_repo_fkey,
    DROP CONSTRAINT ai_reports_repo_org_fkey,
    DROP CONSTRAINT ai_reports_run_requires_repo;

ALTER TABLE build_scores DROP CONSTRAINT build_scores_run_repo_fkey;
ALTER TABLE dora_metrics DROP CONSTRAINT dora_metrics_repo_org_fkey;

ALTER TABLE test_results
    DROP CONSTRAINT test_results_job_run_repo_fkey,
    DROP CONSTRAINT test_results_run_repo_fkey;

ALTER TABLE deployments
    DROP CONSTRAINT deployments_run_repo_fkey,
    DROP CONSTRAINT deployments_commit_repo_fkey;

ALTER TABLE build_logs
    DROP CONSTRAINT build_logs_job_run_repo_fkey,
    DROP CONSTRAINT build_logs_run_repo_fkey;

ALTER TABLE workflow_jobs DROP CONSTRAINT workflow_jobs_run_repo_fkey;

ALTER TABLE workflow_runs
    DROP CONSTRAINT workflow_runs_pr_repo_fkey,
    DROP CONSTRAINT workflow_runs_commit_repo_fkey,
    DROP CONSTRAINT workflow_runs_workflow_repo_fkey;

ALTER TABLE ai_reports DROP CONSTRAINT ai_reports_id_repo_key;
ALTER TABLE workflow_jobs DROP CONSTRAINT workflow_jobs_id_run_repo_key;
ALTER TABLE workflow_runs DROP CONSTRAINT workflow_runs_id_repo_key;
ALTER TABLE pull_requests DROP CONSTRAINT pull_requests_id_repo_key;
ALTER TABLE commits DROP CONSTRAINT commits_id_repo_key;
ALTER TABLE workflows DROP CONSTRAINT workflows_id_repo_key;
ALTER TABLE repositories DROP CONSTRAINT repositories_id_org_key;

DROP INDEX organizations_personal_owner_key;

ALTER TABLE test_results DROP CONSTRAINT test_results_run_job_test_key;
ALTER TABLE test_results
    ADD CONSTRAINT test_results_run_test_key UNIQUE (workflow_run_id, test_key);

ALTER TABLE pull_requests
    ADD CONSTRAINT pull_requests_state_check CHECK (state IN ('open', 'closed'));
