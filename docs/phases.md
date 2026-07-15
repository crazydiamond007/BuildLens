# Build phases

Each phase ships something runnable and is reviewed before the next starts.

| Phase | Scope | Status |
| ----- | ----- | ------ |
| 1 | Foundation: repo layout, compose stack, schema, gateway skeleton | **done** |
| 2 | Auth: GitHub OAuth, sessions, API tokens, user/org/membership endpoints | **done** |
| 3 | Repository sync: GitHub client, repo/branch/commit sync, webhook receiver | next |
| 4 | Workflow ingestion: runs/jobs/steps, log storage to S3, event contracts | |
| 5 | Java analytics: DORA, flaky tests, scoring, scheduled recomputation | |
| 6 | Python AI worker: build summaries and recommendations | |
| 7 | Next.js dashboard | |
| 8 | Testing, hardening, deployment | |

## Decisions made in Phase 1

**Organizations are BuildLens workspaces, not mirrors of GitHub orgs.** Every user
gets a personal org at signup; repositories always belong to exactly one org. This
makes authorization a single rule everywhere. Mirroring GitHub's membership would
mean inheriting GitHub's permission model, in which read access to code implies
read access to metrics. Those are not the same permission.

**Deployments come from both sources, with a `source` column.** Most repositories
never call the GitHub Deployments API; they just run a workflow that happens to
deploy. Ingesting only real Deployment objects gives clean data and empty
dashboards. So we also infer a deployment from a successful default-branch run,
and label which is which, so the UI can be honest about it.

**One migration set, run by a migrator container.** See the README.

**Transactional outbox for publishing to RabbitMQ.** See the README.

**Each re-run attempt is its own `workflow_runs` row**, keyed on
`(github_run_id, run_attempt)`. GitHub reuses the run ID and increments the
attempt. Collapsing them would destroy the signal flaky-test detection depends on:
same commit, same workflow, failed, then passed on retry.

**GitHub-sourced status fields are plain text, not enums.** GitHub adds values to
these over time (`stale`, `action_required`, `startup_failure` all postdate the
API). An enum would mean ingestion throws on a value GitHub invented last Tuesday.
Fields we control get CHECK constraints; fields GitHub controls do not.

**UUIDv7 primary keys.** Time-ordered, so they index like a sequence instead of
scattering writes across the B-tree. This matters on `workflow_runs` and
`test_results`, the two tables that actually get big. And unlike a sequence, all
three languages can generate one before the row exists, which is what makes the
outbox work.

**Percentiles, not averages, in `dora_metrics`.** DORA distributions are
long-tailed. One PR that sat in review over the holidays drags a mean lead time
into uselessness. p50 says what a normal change feels like; p90 says what a bad
week feels like. A mean says neither. `sample_size` is stored alongside, so the
UI can decline to draw a confident line through four data points.

## Deferred, deliberately

**Partitioning.** `workflow_runs`, `test_results` and `audit_logs` will eventually
want range partitioning by time. Partitioning an empty database is speculative and
complicates every query written between now and then. The indexes are designed so
it can be added later without a schema rewrite. Phase 8.

**Pull request reviews.** `pull_requests.first_review_at` exists but is never
populated: filling it needs `pull_request_review` events, which are not yet
ingested. Review latency is the most interesting slice of lead time, so this is
worth doing, but it needs an explicit decision to widen Phase 3's webhook scope.

**Who parses JUnit XML.** `test_results` is currently writable by the gateway, on
the reasoning that parsing test reports means downloading an artifact from GitHub
and the gateway is what holds the credentials. If Phase 5 would rather Java did
the parsing, that is a one-line change in `000010_grants.up.sql`.
