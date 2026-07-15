-- Who is allowed to write what.
--
-- Three services share one Postgres. That is a shared-database architecture no
-- matter how we package it, and pretending otherwise by giving each service its
-- own migration tool would not make it separate. It would just mean nobody
-- could read the schema without opening three directories. So instead: one
-- schema, one migrator, and privileges that make the ownership boundary real
-- rather than a convention people remember on a good day.
--
-- THE RULE: a service may write the tables it owns and read everything else.
--
-- The line between owners is facts versus derivations.
--
--   gateway   (Rust): talks to GitHub, so it owns every observed fact.
--   analytics (Java): computes, so it owns every derived number.
--   ai        (Python): generates, so it owns its own reports.
--
-- The one that could reasonably move is test_results. It sits with the gateway
-- because parsing JUnit XML means downloading an artifact from GitHub, and the
-- gateway is what holds the GitHub credentials. If Phase 5 wants Java to do
-- that parsing instead, this is a one-line change here.
--
-- Roles are created in infra/postgres/init/01-roles.sh (a migration cannot hold
-- a password), which also grants CONNECT, USAGE, and default SELECT.

-- Reads are shared. All three services can see the whole picture; this is an
-- analytics product, and an analytics service that cannot read the commits it is
-- computing lead time from is not useful.
GRANT SELECT ON ALL TABLES IN SCHEMA public TO buildlens_services;

-- ---------------------------------------------------------------------------
-- gateway (Rust): the facts
-- ---------------------------------------------------------------------------
GRANT INSERT, UPDATE, DELETE ON
    users,
    github_accounts,
    organizations,
    organization_members,
    api_tokens,
    repositories,
    repository_sync_state,
    branches,
    commits,
    pull_requests,
    workflows,
    workflow_runs,
    workflow_jobs,
    workflow_steps,
    build_logs,
    deployments,
    test_results,
    webhook_deliveries,
    event_outbox
TO buildlens_gateway;

-- ---------------------------------------------------------------------------
-- analytics (Java): the derivations
-- ---------------------------------------------------------------------------
GRANT INSERT, UPDATE, DELETE ON
    dora_metrics,
    repository_scores,
    build_scores,
    flaky_tests
TO buildlens_analytics;

-- The analytics service recomputes from scratch when a metric definition
-- changes. It needs to be able to clear its own tables to do that, and only
-- its own.

-- ---------------------------------------------------------------------------
-- ai (Python): its own output
-- ---------------------------------------------------------------------------
GRANT INSERT, UPDATE, DELETE ON
    ai_reports,
    ai_recommendations
TO buildlens_ai;

-- ---------------------------------------------------------------------------
-- Shared write surfaces
-- ---------------------------------------------------------------------------

-- Any service may notify a user. Only the gateway may mark one read, because
-- only the gateway serves the API the user clicks.
GRANT INSERT ON notifications TO buildlens_services;
GRANT UPDATE, DELETE ON notifications TO buildlens_gateway;

-- Any service may append to the audit log. NOBODY may update or delete from it,
-- including the gateway. An audit log you can edit is not an audit log.
GRANT INSERT ON audit_logs TO buildlens_services;
