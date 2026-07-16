# Build phases

Each phase ships something runnable and is reviewed before the next starts.

| Phase | Scope | Status |
| ----- | ----- | ------ |
| 1 | Foundation: repo layout, compose stack, schema, gateway skeleton | **done** |
| 2 | Auth: GitHub OAuth, sessions, API tokens, user/org/membership endpoints | **done** |
| 3 | Repository sync: GitHub client, repo/branch/commit sync, webhook receiver | **done** |
| 4 | Workflow ingestion: runs/jobs/steps, log storage to S3, event contracts, outbox relay | **done** |
| 5 | Java analytics: DORA, flaky tests, scoring, scheduled recomputation | **done** |
| 6 | Python AI worker: build summaries and recommendations | **done** |
| 7 | Next.js dashboard | **in review** |
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

## Decisions made in Phase 6

**Paid idempotency gets a receipt ledger.** Java analytics can safely recompute a
local upsert, but an AI replay can charge the account twice. Migration 12 therefore
adds `ai_event_receipts.event_id` as the strict message-level claim, while the
existing `(workflow_run_id, kind)` unique index remains the business-level guard.
The report is claimed before model input is counted or generated.

**Model spend is admitted under a database lock.** Failure analysis uses
`claude-opus-4-8`; summaries and digests use `claude-haiku-4-5`. The worker counts
input first, reserves the worst normal cache-write plus maximum-output cost, and
holds a Postgres advisory lock from the monthly spend check through report commit.
This makes the default `$10` ceiling hard across multiple worker replicas.

**Generation is opt-in where it scales with traffic.** Failed-run analysis is the
core trigger. Successful-build summaries and scheduled repository reports exist
but default off, because enabling either changes unit economics merely by traffic
volume or elapsed time.

**Only bounded, redacted evidence leaves the box.** The model receives structured
database facts and small failure-centered log ranges. It never receives raw log
archives, and the worker does not fetch repository source files. Archive expansion and prompt size are bounded, every
outbound string is secret-redacted, and every model citation is checked against
the supplied IDs/ranges before the report commits.

## Decisions made in Phase 7

**The frontend talks only to the gateway.** Next.js Server Components forward the
opaque session cookie to authenticated, organization-scoped read endpoints. The
browser never receives database, RabbitMQ, MinIO, GitHub, or AI provider
credentials, and the frontend has no service database role.

**Dashboard responses are narrow read models.** The gateway joins observed facts,
analytics derivations, and AI output after enforcing the existing `viewer` role.
This keeps authorization in one place and avoids duplicating the shared schema in
a browser-facing query layer. The schema and grants did not change.

**Recommendation state remains read-only.** The gateway cannot update AI-owned
tables, and Phase 7 does not weaken that grant boundary merely to support an
acknowledge button. A future status workflow needs an explicit ownership and audit
decision before it gains a mutation endpoint.

**The design artifact is a specification, not a runtime.** Its Hanken Grotesk and
JetBrains Mono typography, OKLCH themes, dense application shell, semantic states,
and responsive layouts were rebuilt as typed React components and CSS. The
prototype support runtime is not shipped.

## Deferred, deliberately

**Partitioning.** `workflow_runs`, `test_results` and `audit_logs` will eventually
want range partitioning by time. Partitioning an empty database is speculative and
complicates every query written between now and then. The indexes are designed so
it can be added later without a schema rewrite. Phase 8.

**Who parses JUnit XML.** Decided in Phase 5: the gateway downloads bounded run
artifacts and parses JUnit XML after `workflow_run.completed`, because it already
holds GitHub credentials and owns observed facts. Analytics remains a pure
Postgres + RabbitMQ consumer. The scheduled analytics pass closes the intentional
race between immediate events and best-effort post-commit artifact capture.

**Real GitHub Deployment API ingestion.** Phase 4 records deployments only by
*inference* - a successful `push`/`release`/`workflow_dispatch` run on the default
branch becomes a `workflow_inferred` production deployment. The schema already
carries a `github_deployment` source and the gateway subscribes to
`deployment_status` webhooks, but ingesting real Deployment objects is not wired
yet. It is additive (a new source label, a new event branch) and can land in
Phase 4.x or alongside Phase 5 without a schema change.
