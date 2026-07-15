# BuildLens  agent handoff

Read this before changing anything. It describes what exists, what is load-bearing
about it, and what the next phase is.

BuildLens is a DevOps analytics platform for GitHub Actions. Rust ingests, Java
analyses, Python generates, Next.js displays. They meet at a message bus and a
shared Postgres.

---

## Status: Phase 6 implemented on its feature branch and pending owner review.

Phase 1 delivered infrastructure and the full database schema. Phase 2 delivered
GitHub OAuth, Redis sessions, API tokens, unified authentication, organization and
membership APIs, and audit logging - committed on `feat/phase-2-auth` and verified
end-to-end against the live stack: health, auth gating, the session and
`Bearer` paths, organization authorization, audit writes, and the per-role grant
boundary (the gateway can INSERT `organizations` but cannot INSERT `dora_metrics`
or UPDATE `audit_logs`) all hold. The one path not exercised at runtime is the real
GitHub OAuth round-trip, which needs a registered OAuth App's credentials.

Phase 3 is merged (repository discovery and opt-in tracking, automatic webhook
registration, resumable branch/commit/PR sync, signed webhook persistence, and
background processing for `push`, `pull_request`, and `pull_request_review`).

Phase 4 is merged: workflow ingestion (runs/jobs/steps,
per-attempt), workflow-run backfill, build-log storage to MinIO, inferred
deployments, the event contract in `contracts/`, and - the headline - the
transactional-outbox relay that publishes to RabbitMQ. Verified end-to-end against
the live stack: a signed `workflow_run` webhook produces `workflow_runs` /
`workflow_jobs` / `workflow_steps` rows, an inferred `deployments` row, two
`event_outbox` rows written in the ingest transaction, and the relay publishes both
to the `buildlens.events` topic exchange with publisher confirms - a bound test
queue received both messages, each carrying the envelope with its `message_id`
idempotency key. A duplicate run emitted no new events (idempotent emission).
Since verified against a **real** repository: tracking `crazydiamond007/Webhook`
backfilled 12 branches, 48 commits, 11 PRs, 1 workflow, 38 runs, 152 jobs, 1317
steps and 8 inferred deployments, and the relay published all 46 events. The one
path still unconfirmed on real data is webhook-time log capture on a *live* run
(backfill deliberately does not fetch logs).

Phase 5 is merged: a Java 25 / Spring Boot
4.1 analytics service consumes the two thin events, validates the migration-owned
schema, and writes DORA rollups, build/repository scores, and flaky-test verdicts.
The gateway now parses bounded JUnit artifacts into `test_results`. Live-stack
verification covered Postgres 18.4 validation, RabbitMQ queue/bindings/consumer,
valid workflow and deployment triggers, duplicate recomputation with unchanged row
counts, unknown-version dead-lettering, the Compose container health check, and the
analytics role grant boundary. No tracked real run currently exposes a JUnit
artifact, so artifact capture is covered by parser tests but is not yet confirmed
end-to-end against GitHub.

Phase 6 is implemented on `feat/phase-6-ai-worker`: a UV-managed Python 3.13 /
FastAPI service consumes the same thin events, reads facts/derivations from
Postgres and bounded failure excerpts from MinIO, and writes structured grounded
reports and recommendations. Migration 12 adds its event-ID receipt ledger because
ordinary at-least-once redelivery cannot be allowed to duplicate a paid model call.
Live-stack verification covered Postgres 18 migration/grants, the durable RabbitMQ
queue and DLX, valid duplicate no-op processing with one receipt and zero reports,
unknown-version dead-lettering, container health/readiness, and the manual endpoint
auth boundary. Seventeen unit tests cover config, event validation, grounding, redaction, and
cost accounting. A real Anthropic generation is not yet exercised because the
local `.env` has no API key. **Phases 7-8 do not exist yet:** no frontend or
hardening phase. Phase 7 starts only after owner review and Phase 6 merge.

Phase list and the reasoning behind each Phase 1 decision: `docs/phases.md`.

---

## How to work here

**One phase at a time.** Each phase ships something runnable and is reviewed by the
owner before the next starts. Do not scaffold later phases early. An empty
directory with a README is the correct state for work that has not been approved.

**Explain design decisions as you go.** The owner wants to understand the system,
not receive finished code. Where the Rust/Java/Python boundary or an event contract
needs a judgement call, surface it rather than silently picking.

**Keep changes scoped to the current phase.** Phases 1-5 are merged. Phase 6 is in
review. Work each phase on its own branch (`feat/phase-N-...`) and open it for
review before it merges - the owner reviews, with Claude, before the next phase
starts.

---

## Invariants: do not break these without an explicit decision

**1. The schema is owned by `infra/migrations`. No service migrates.**
Three services share one Postgres. One migrator container (`migrate/migrate`) runs
`infra/migrations/*.up.sql` to completion before any service starts. When Java
arrives it runs `spring.jpa.hibernate.ddl-auto=validate` with Flyway **disabled**;
Python never migrates. New migration: `make migrate-create NAME=whatever`.

**2. Per-service Postgres roles enforce table ownership.**
The rule is *facts versus derivations*. The gateway talks to GitHub, so it owns
every observed fact. Analytics computes, so it owns every derived number. The AI
worker generates, so it owns its reports. Everyone reads everything. This is
enforced by the initial grants in `000010_grants.up.sql` and grants added alongside
later tables, not by convention. Analytics
physically cannot insert into `workflow_runs`, and **nobody** can UPDATE or DELETE
`audit_logs`. If you add a table, add its grant.

**3. Foreign keys across event streams are nullable on purpose.**
GitHub webhooks arrive out of order and at-least-once. A `workflow_run.completed`
routinely lands before the `push` that created its commit. So runs store `head_sha`
as text and carry a nullable `head_commit_id` backfilled later. Making those FKs
strict deadlocks ingestion against itself. Within a stream (job → run, step → job)
GitHub guarantees ordering, so those FKs *are* strict. Do not "fix" the nullable
ones.

**4. GitHub-sourced status fields are plain TEXT, not enums.**
GitHub adds values over time (`stale`, `action_required`, `startup_failure` all
postdate the API). An enum means ingestion throws on a value GitHub invented last
week. Fields *we* control (`role`, `severity`, `granularity`) get CHECK constraints.
Fields GitHub controls do not.

**5. Events publish through a transactional outbox.**
The gateway writes the row and an `event_outbox` row in one Postgres transaction; a
relay publishes to RabbitMQ afterwards and marks it published. Delivery is
therefore **at-least-once**, so every consumer must be idempotent. Duplicates are
survivable; silent loss is not. The relay lives in `gateway/src/relay.rs`; the
wire contract every consumer binds to is `contracts/`.

**6. Every re-run attempt is its own `workflow_runs` row.**
Keyed `(github_run_id, run_attempt)`. GitHub reuses the run ID and increments the
attempt. Collapsing them destroys the signal flaky-test detection depends on: same
commit, same workflow, failed, then passed on retry.

**7. Primary keys are `uuidv7()`, generated by the database default.**
Time-ordered, so they index like a sequence. Requires **Postgres 18** (native
`uuidv7()`), which `docker-compose.yml` pins.

---

## Running it

```bash
make env      # .env from .env.example (once)
make up       # postgres, redis, rabbitmq, minio + migrations
make dev      # gateway on the host against the dockerised stack

curl localhost:8080/health         # liveness
curl localhost:8080/health/ready   # readiness (checks postgres + redis)
```

`make help` lists the rest. `make reset` destroys all volumes.

**Postgres is published on host port 5433, not 5432.** A system-installed Postgres
on the developer's machine occupies 5432 and shadows the container. Inside the
compose network it is still `postgres:5432`. If you see `password authentication
failed for user "buildlens_gateway"`, you are talking to the wrong database.
Postgres reports an unknown role that way.

---

## What exists

```
gateway/          Rust + Axum. Auth, GitHub API, sync, and webhook ingestion.
  src/main.rs       startup, tracing, graceful shutdown, webhook worker lifecycle
  src/config.rs     env parsing and validation; fails loudly at boot
  src/state.rs      Postgres, Redis, HTTP client, encryption, config
  src/auth.rs       session/API-token extractor and organization authorization
  src/sessions.rs   opaque Redis sessions and one-time OAuth state
  src/github.rs     OAuth exchange and GitHub identity lookup
  src/github_api.rs authenticated REST client, pagination, ETags, rate limits, hooks
  src/repository_sync.rs resumable branches, commits, PRs, reviews, workflows, runs
  src/webhooks.rs   signature verification, persistence, background apply (incl. runs/jobs)
  src/workflow_ingest.rs runs/jobs/steps upserts, deployment inference, event emission
  src/events.rs     the outbox writer and the event envelope
  src/relay.rs      the outbox -> RabbitMQ relay (topology, confirms, backoff)
  src/logs.rs       build-log storage to S3/MinIO (best-effort, off the hot path)
  src/junit.rs      bounded JUnit artifact parsing into test_results
  src/routes/       auth/tenancy/tokens plus repository discovery and tracking
infra/migrations/ 12 migrations, 28 application tables. The schema source of truth.
infra/postgres/   init/01-roles.sh: creates the 3 service roles (passwords can't
                  live in a migration, so role creation cannot either)
infra/minio/      bootstrap.sh: creates the buckets
contracts/        the event envelope, per-event examples, topology, versioning
analytics/        Java 25 + Spring Boot. Rabbit consumer and derived analytics.
ai-worker/        Python + FastAPI + UV. Grounded AI reports and recommendations.
frontend/         empty. Phase 7.
docs/phases.md    the decision log; read it
```

**Gateway conventions worth matching.** `sqlx` is used *without* the `macros`
feature on purpose: `query!` validates against a live database at compile time,
which would make `cargo build` require a running Postgres. Use the runtime query
API. Liveness (`/health`) deliberately checks nothing  if it checked Postgres, a
database blip would restart every replica and stampede the database as it
recovered. Readiness (`/health/ready`) checks dependencies and returns 503 with a
per-dependency breakdown. Keep that split.

The Phase 2 principal extractor accepts the `buildlens_session` cookie or a
`Bearer blq_...` token. API tokens are read-only in Phase 2; sensitive account and
membership mutations require a session. Personal organizations cannot accept
extra members. Team organizations serialize membership mutations by locking the
organization row, and they must always retain an owner.

Gate the gateway on `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, and `cargo test`. Gate analytics on `mvn test package`; all pass today.

---

## Phase 2: Auth & GitHub OAuth (Rust). Delivered.

The OAuth flow, opaque Redis sessions (httpOnly cookie, revocable - not a JWT),
AES-GCM-encrypted GitHub tokens, hashed API tokens with a clear lookup prefix, the
`owner|admin|member|viewer` org model with a personal org per user, and audit
logging are all in. The API surface is in `README.md`; the load-bearing decisions
are summarised under "What exists" and enforced by the invariants above. Config
added: `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `GITHUB_REDIRECT_URI`,
`TOKEN_ENCRYPTION_KEY`, `SESSION_TTL_SECONDS`.

---

## Phase 3: Repository sync & webhooks (Rust). Delivered.

Phase 3 is where facts start flowing out of GitHub. Phase 2 left us a logged-in
user with an encrypted OAuth token; Phase 3 uses it to populate the repository side
of the schema - `repositories`, `repository_sync_state`, `branches`, `commits`,
`pull_requests` - and stands up the webhook receiver that keeps them current. It is
**Rust-only**. No Java, no Python, no RabbitMQ yet.

### Decisions locked by the owner

**GitHub is called as the user, with their decrypted OAuth token** - not a GitHub
App installation token. The Phase 2 scopes (`read:user user:email repo read:org`)
already cover reading repos and managing repo hooks. A GitHub App would give higher
rate limits and per-install webhooks, at the cost of a much larger setup; for a
portfolio project the user-token path is the pragmatic call. **Boundary flag:** if
you ever want org-wide install without each user having admin, that is the App
route - decide now, because it changes the webhook story below.

**Conditional requests and rate-limit handling are mandatory, not polish.** That is
the entire reason `repository_sync_state` carries an `etag` and a `cursor` per
resource. Send `If-None-Match`; a `304` costs no rate budget and means "nothing
changed". Honour `X-RateLimit-Remaining` / `Reset` and back off on secondary limits.
A sync that re-walks full history every run will exhaust the hourly limit.

**Sync is opt-in per repo.** `repositories.tracking_enabled` exists for this.
Listing a user's repos is cheap; syncing every repo they can see is rude to GitHub
and useless on the dashboard. The user picks which repos to track; only tracked
repos sync and only tracked repos have their webhooks processed.

**Verify the webhook signature before parsing a single byte.** HMAC-SHA256 over the
**raw** request body with the shared secret, constant-time compare against
`X-Hub-Signature-256`. But record the delivery either way:
`webhook_deliveries.signature_valid` is stored, not enforced by the table, because a
run of bad signatures is a security signal and deleting it erases the evidence.

**Webhook idempotency is the `webhook_deliveries.github_delivery_id` UNIQUE
constraint.** Insert the delivery first; a replayed `X-GitHub-Delivery` collides and
is dropped instead of double-counting. The table doubles as a replay log - the fix
for a processing bug is to correct the code and re-drive the stored payloads, not to
beg GitHub to resend.

**Receive fast, process off the hot path.** The handler's job is verify → persist
the raw delivery → return `2xx` quickly so GitHub doesn't time out and retry. Turning
a delivery into `branches` / `commits` / `pull_requests` rows happens separately;
`webhook_deliveries_pending_idx` is literally "the work queue for the webhook
processor". A single-process background task draining that queue is fine for Phase 3.

**Boundary flag - the outbox stays empty in Phase 3.** The gateway *has* the grant
to write `event_outbox`, but the relay that publishes it is Phase 4. Do not start
writing outbox rows now; they would just pile up unpublished. Phase 3 gets facts
into Postgres and no further. Event contracts and publishing are Phase 4's job.

### Delivered scope

- **A GitHub REST client** (extend `src/github.rs` or add a module): authenticated
  as the user via the decrypted token, with pagination, `ETag` conditional requests,
  and rate-limit handling. Typed response structs, same style as the Phase 2 OAuth
  types.
- **Repo discovery + tracking.** List the user's GitHub repos (joined against what
  is already tracked); a mutation to start/stop tracking a repo, which writes the
  `repositories` row into the caller's chosen organization and flips
  `tracking_enabled`. Tracking is session-only and requires the BuildLens `admin`
  role plus GitHub repository admin permission.
- **Initial sync** for a tracked repo: backfill `branches`, `commits`,
  `pull_requests`, paginated and resumable via the `repository_sync_state`
  cursors/etags, updating `sync_status` / `last_synced_at` / `last_error` as it goes.
- **Webhook receiver:** `POST /webhooks/github` - verify signature, dedupe on
  `github_delivery_id`, persist to `webhook_deliveries`, return `2xx`.
- **Webhook processor:** drain pending deliveries and apply `push`,
  `pull_request`, and `pull_request_review` events to branches, commits, PRs, and
  `first_review_at`. Resolve the soft `repository_id` from `github_repo_id`; a
  webhook for an untracked or not-yet-synced repo is recorded and ignored.
- **Audit + authorization.** Reads gated by org membership; tracking changes audited
  (`repository.tracking_enabled` / `.disabled`) with the existing `audit::write`.

### Config added

- `GITHUB_WEBHOOK_SECRET` - the HMAC secret for signature verification.
- `GITHUB_WEBHOOK_URL` - the public callback registered on tracked repositories.
- `GITHUB_API_BASE_URL` - defaults to `https://api.github.com` and can target a
  test server.

### Deliberately absent (do NOT build in Phase 3)

`workflow_runs` / `jobs` / `steps` ingestion and build-log storage to MinIO
(Phase 4). Deployment ingestion (depends on workflow runs - Phase 4+). RabbitMQ
publishing and the outbox relay (Phase 4). DORA / scoring / flaky-test analytics
(Phase 5, Java). Anything Python or frontend.

---

## Decisions recorded during Phase 3

**`pull_request_review` events are included.** Initial sync backfills the first
review timestamp and webhook processing keeps it current.

**Webhooks are registered automatically.** Tracking requires a BuildLens `admin`
and GitHub repository admin permission. Registration failures are returned to the
caller; the repository is not marked tracked when its hook is absent.

**Who parses JUnit XML into `test_results`?** This was resolved in Phase 5: the
gateway downloads and parses bounded JUnit artifacts because it already holds the
GitHub credentials. The existing grant therefore remains unchanged.

---

## Phase 4: Workflow ingestion, log storage & the event relay (Rust). Delivered.

Phase 4 turns builds into facts and stands up the message bus. Still **Rust-only** -
no Java or Python - but it defines the contract those services will consume.

### Decisions made in Phase 4

**Events are triggers, not the source of truth.** Postgres is. An event says "repo
X has a new completed run - recompute", carrying only routing + an idempotency key;
consumers read Postgres for detail. This keeps the wire format small and stable.
The full contract (envelope, events, topology, versioning, idempotency) is in
`contracts/`. **Boundary flag:** this is the Rust→Java/Python seam. If Phase 5 wants
events to carry the computed detail rather than a pointer, that is a contract
change to make now, before consumers exist.

**Two events ship in Phase 4:** `workflow_run.completed` and `deployment.recorded`,
emitted only on the *transition* into that state so a replayed webhook does not
re-fire them. The envelope reserves room for `push.*` / `pull_request.*` without a
version bump.

**Deployments are inferred, not yet ingested from the Deployment API.** A
successful `push`/`release`/`workflow_dispatch` run on the default branch becomes a
`workflow_inferred` production deployment (idempotent via the partial unique index).
Real GitHub Deployment ingestion is additive and deferred - see `docs/phases.md`.

**Log storage is best-effort and off the hot path.** On a completed run the
processor spawns a task that downloads the run's log zip and stores it to MinIO;
any failure is logged and dropped, never blocking the facts that already committed.
The run log is stored as-is (zipped); unpacking is a consumer's job.

**The relay borrows a member's GitHub token for webhook-time log capture.** A
webhook has no session, so `github_api::repository_token` picks any org member with
a valid token. **Boundary flag:** a production system would use a GitHub App
installation token instead; this is a deliberate portfolio-scope simplification.

### Delivered scope

- **Workflow ingestion** - `workflow_run` and `workflow_job` webhooks and a
  resumable REST backfill (`workflows`, then `workflow_runs` with their jobs/steps)
  populate `workflows` / `workflow_runs` / `workflow_jobs` / `workflow_steps`,
  per-attempt, with soft head-commit / PR FK resolution.
- **The outbox relay** (`relay.rs`) - declares the `buildlens.events` topic
  exchange and a DLX, drains `event_outbox` with `FOR UPDATE SKIP LOCKED`,
  publishes with confirms, marks published only on ack, backs off and dead-ends
  after `MAX_ATTEMPTS`, and reconnects on connection loss.
- **Inferred deployments**, **build-log storage to MinIO**, and the **event
  contract** in `contracts/`.

### Config added

`RABBITMQ_URL`, and `S3_ENDPOINT` / `S3_REGION` / `S3_ACCESS_KEY` / `S3_SECRET_KEY`
/ `S3_LOGS_BUCKET`. All required at boot. `docker-compose.yml` maps them from the
existing RabbitMQ/MinIO vars; the gateway now depends on both being healthy.

### Deliberately absent when Phase 4 shipped

DORA / scoring / flaky-test analytics (Phase 5, Java - it reads these facts and the
events). JUnit parsing into `test_results`. Real GitHub Deployment API ingestion.
Anything Python or frontend.

---

## Phase 5: DORA, scoring & flaky-test analytics (Java). Implemented, pending review.

Phase 5 is the **first non-Rust service and the first consumer**. The gateway now
produces facts (in Postgres) and events (on RabbitMQ). Analytics turns them into the
derived numbers the dashboard shows: DORA, build/repo scores, flaky tests. It is
Spring Boot, and it writes **only** the derivation tables - nothing it touches is a
fact from GitHub. **This is the Rust→Java boundary; the calls below are the ones the
owner confirmed before implementation.**

### Decisions locked in

**Analytics reads facts and writes only derivations - the grants already enforce
it.** The service connects as the `buildlens_analytics` role and can `INSERT` /
`UPDATE` / `DELETE` exactly four tables: `dora_metrics`, `repository_scores`,
`build_scores`, `flaky_tests` (`000010_grants.up.sql`). It **physically cannot**
write `workflow_runs` or any other fact. Do not work around this; it is invariant #2.

**The schema is validate-only.** `spring.jpa.hibernate.ddl-auto=validate`, and
Flyway/Liquibase **disabled** (invariant #1). `infra/migrations` owns the schema.
Map JPA entities onto the existing tables; if an entity does not match, fix the
entity, not the database. A genuinely new table goes
through `make migrate-create` **with a grant**, never a service-side migration.

**Events are triggers; Postgres is the source of truth.** Consume
`workflow_run.completed` and `deployment.recorded`, validate that the AMQP
`message_id` matches the envelope `id`, then read the facts from Postgres and
recompute. Delivery is at-least-once (invariant #5), so duplicate envelopes must
produce the same natural-key rows. Do not compute from the event payload alone.

**The consumer owns its topology.** Declare a durable queue (e.g.
`analytics.workflow_runs`) bound to `buildlens.events` on `workflow_run.*` and
`deployment.*`, with `x-dead-letter-exchange = buildlens.events.dlx`. The gateway
declares only the exchanges. Ack a message only after its row is committed (or it is
dead-lettered). A `version` the consumer does not understand is dead-lettered, not
crashed on.

**Recompute is natural-key replacement, so it is idempotent by construction.**
Every metric is a pure function of the facts. Write it as an UPSERT on the natural
key - the two partial unique indexes on `dora_metrics`, the `UNIQUE`
`build_scores.workflow_run_id`, `flaky_tests (repository_id, test_key)`. A duplicate
event recomputes the same row; never append. Because of this, a separate
processed-events ledger is probably unnecessary - flag it if you think strict
once-only side effects are needed, because that ledger would need a migration + grant.

**Percentiles, not means.** `dora_metrics` stores `lead_time_p50/p90`, `mttr_p50/p90`
and `sample_size`. Compute distributions and populate `sample_size` honestly - the UI
uses it to decline drawing a confident line through four data points.

### Delivered scope

- **A Maven Spring Boot service in `analytics/`**, connecting as
  `buildlens_analytics`, `ddl-auto=validate`, added to `docker-compose.yml` under the
  `app` profile with `depends_on` postgres/rabbitmq healthy + migrator completed.
- **A Spring AMQP consumer** on its own queue, validating the AMQP/envelope
  identity, recomputing idempotently, and dead-lettering poison / unknown-version
  messages.
- **DORA → `dora_metrics`**, per repo and per-org rollup, at daily/weekly/monthly
  granularity: deployment frequency (from `deployments`), lead time (commit
  `authored_at` → deployment `deployed_at`), change failure rate + MTTR, plus
  `performance_band` and `sample_size`.
- **Flaky-test detection → `flaky_tests`**: flips between run attempts on an
  unchanged commit in the same workflow, after collapsing matrix-job outcomes
  per run. Flake rate divides by comparable retry transitions.
- **Scoring → `build_scores`** (one row per run) **and `repository_scores`**
  (trailing window, history retained).
- **Scheduled recompute** (`@Scheduled`) to roll trailing windows forward and cover
  periods the event stream did not touch.

### Config added

`ANALYTICS_DB_PASSWORD` and the `buildlens_analytics` `DATABASE_URL` (the role and
password already exist), and `RABBITMQ_URL` (same broker). Analytics needs no S3 or
GitHub credentials because the gateway owns artifact ingestion.

### Decisions resolved during Phase 5

- **The gateway parses JUnit.** On a completed live run it lists GitHub artifacts,
  applies archive/XML size and count ceilings, parses JUnit off the webhook task,
  and upserts `test_results`. Analytics receives no GitHub or S3 credentials.
- **Lead time uses the deployed commit's `authored_at`.** Resolve the direct
  `commit_id` first, then `(repository_id, sha)`. An unresolved commit remains in
  deployment frequency/CFR and is excluded only from the lead-time distribution.
- **Events stay thin.** Postgres remains the source of truth; the existing
  `contracts/` envelope did not change.
- **No processed-event ledger.** All side effects are deterministic natural-key
  upserts. Duplicate delivery recomputes the same rows; strict once-only side
  effects would justify a ledger later, but none exist in Phase 5.

### Deliberately absent (do NOT build in Phase 5)

The Python AI worker (Phase 6) and the Next.js frontend (Phase 7). No new gateway
features beyond the agreed JUnit artifact seam. If analytics needs another fact
that is not ingested yet, flag it back to a gateway change; do not synthesise it.
No Phase 5 schema migration was needed.

---

## Phase 6: AI build summaries & recommendations (Python). Implemented, pending review.

Phase 6 is the **first Python service and the third consumer**. Rust produced the
facts, Java derived the numbers; Phase 6 *explains* them. It consumes the same
events, reads facts from Postgres and build logs from MinIO, calls an LLM, and
writes `ai_reports` and `ai_recommendations`. FastAPI is the shell (health +
manual re-trigger); the RabbitMQ consumer is the substance.

### Decisions locked in

**It owns only its reports, recommendations, and receipt ledger - the grants
enforce it.** The service connects as `buildlens_ai`; migration 12 grants it writes
to `ai_event_receipts`, alongside the existing `ai_reports` and
`ai_recommendations` grants. It reads every fact/derivation and physically cannot
write `workflow_runs` or audit history. Python **never migrates** (invariant #1).

**Idempotency is a *paid* concern here - a duplicate event is a duplicate LLM
bill.** The `ai_reports (workflow_run_id, kind)` partial UNIQUE is the guard:
`INSERT ... ON CONFLICT DO NOTHING` a `pending` row *first*, and only call the
model if you won the insert. Migration 12's `ai_event_receipts.event_id` UNIQUE
also deduplicates no-op and repository-level triggers. Never regenerate a completed
report on replay. A crash after the provider responds but before Postgres commits
is the unavoidable external-side-effect window; stale claims recover rather than
silently lose the report.

**Consumer owns its topology.** Declare a durable queue (e.g. `ai.reports`) bound
to `buildlens.events` on `workflow_run.*` / `deployment.*`, dead-lettered to
`buildlens.events.dlx`. The gateway declares only the exchanges. Ack only after the
report row is committed (or the message is dead-lettered).

**Output must be grounded and checkable.** `ai_reports.content` (jsonb) and
`ai_recommendations.evidence` (jsonb) carry the cited job/step ids, failing
`test_key`s, and log line ranges the model looked at. Use **structured outputs**
(`client.messages.parse(...)` with a Pydantic model) so the model returns exactly
the JSON these columns expect - not prose you then regex.

**Provenance and unit economics are columns, so populate them every call.**
`ai_reports` has `model`, `prompt_version`, `input_tokens`, `output_tokens`,
`cost_usd`, `latency_ms`. Read `response.usage` for the token counts; count ahead
of time with `client.messages.count_tokens(...)`, **never tiktoken** (it undercounts
Claude). A prompt change stays attributable, and cost stays visible.

**The owner chose the tiers and hard cap.** `claude-opus-4-8` handles
`failure_analysis` with adaptive thinking; `claude-haiku-4-5` handles summaries
and digests. Shared prompt/context blocks use ephemeral prompt caching. The worker
counts tokens before generation, projects the worst normal cache-write/output
cost, and serializes admission under a Postgres advisory lock against a hard
`$10.00` monthly ceiling. Billable failed attempts accumulate into the report's
accounting totals; unknown model prices fail closed.

**Data egress is deliberately narrow and approved.** Only structured facts and
bounded failing-log excerpts go to Anthropic; raw archives are not sent and the
worker never fetches repository source files. Every outbound string, including
JUnit failure messages, passes the secret redactor. The owner accepted Anthropic's
standard retention terms. Log archives
are guarded by download, entry-count, per-entry, uncompressed-size, line-count,
and prompt-byte ceilings plus zip traversal checks.

### Delivered scope

- **A UV-managed Python 3.13 service in `ai-worker/`** (FastAPI for liveness,
  dependency readiness, and bearer-protected manual retry; `aio-pika` consumer),
  connecting as `buildlens_ai`, added to `docker-compose.yml` under the `app`
  profile with `depends_on` postgres/rabbitmq healthy + migrator completed.
- **`failure_analysis`** on a failed `workflow_run.completed`: pull the run, its
  jobs/steps, and failing `test_results` from Postgres (the gateway now populates
  them) + the run's log zip from MinIO, call the model, write one `ai_reports` row
  (`kind = failure_analysis`) and any `ai_recommendations`.
- **`build_summary`** on success, feature-flagged off by default to avoid
  unintentional per-run spend.
- **Scheduled `weekly_digest` / `repo_health`**, feature-flagged off by default,
  reading Phase 5's `dora_metrics` / `flaky_tests` / `*_scores`, plus stale
  pending/processing claim recovery.
- Idempotent on `(workflow_run_id, kind)` and envelope id; poison/oversized /
  unknown-version messages dead-lettered.
- Structured output grounding validates every cited job, step, test, log range,
  and metric key against the supplied context before committing.

### Config added

`ANTHROPIC_API_KEY`, `AI_MANUAL_TRIGGER_TOKEN`, `AI_FAILURE_MODEL`,
`AI_SUMMARY_MODEL`, `AI_MONTHLY_COST_CAP_USD`,
`AI_SUCCESS_SUMMARIES_ENABLED`, `AI_SCHEDULED_REPORTS_ENABLED`, `AI_DB_PASSWORD` +
the `buildlens_ai` `DATABASE_URL`, `RABBITMQ_URL`, and S3/MinIO read credentials.
Dependency and secret values are required and validated at boot.

### Decisions resolved during Phase 6

- **Use UV, not Poetry.** The lock file is committed and CI uses
  `uv sync --frozen` plus Ruff and pytest.
- **Opus for failures, Haiku for lower-cost summaries, `$10/month` hard cap.**
- **Do not summarize every successful run by default.** It is opt-in via env.
- **Scheduled reports are implemented but opt-in.** Startup/smoke tests cannot
  create them just because metrics exist.
- **Use a receipt ledger.** Paid event-ID idempotency justified migration 12 and
  its explicit grant; Java analytics still needs no ledger because its effects are
  deterministic local upserts.
- **No raw archives or fetched source leave the box.** Only bounded, recursively
  redacted context is sent under the accepted standard Anthropic retention policy.

### Deliberately absent (do NOT build in Phase 6)

The Next.js frontend (Phase 7) and hardening (Phase 8). No gateway or analytics
changes - if the worker needs a fact that is not ingested or derived yet, flag it
back, do not synthesise it.
