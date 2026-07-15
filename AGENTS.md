# BuildLens  agent handoff

Read this before changing anything. It describes what exists, what is load-bearing
about it, and what the next phase is.

BuildLens is a DevOps analytics platform for GitHub Actions. Rust ingests, Java
analyses, Python generates, Next.js displays. They meet at a message bus and a
shared Postgres.

---

## Status: Phase 4 implemented on its feature branch and pending owner review.

Phase 1 delivered infrastructure and the full database schema. Phase 2 delivered
GitHub OAuth, Redis sessions, API tokens, unified authentication, organization and
membership APIs, and audit logging — committed on `feat/phase-2-auth` and verified
end-to-end against the live stack: health, auth gating, the session and
`Bearer` paths, organization authorization, audit writes, and the per-role grant
boundary (the gateway can INSERT `organizations` but cannot INSERT `dora_metrics`
or UPDATE `audit_logs`) all hold. The one path not exercised at runtime is the real
GitHub OAuth round-trip, which needs a registered OAuth App's credentials.

Phase 3 is merged (repository discovery and opt-in tracking, automatic webhook
registration, resumable branch/commit/PR sync, signed webhook persistence, and
background processing for `push`, `pull_request`, and `pull_request_review`).

Phase 4 is implemented on `feat/phase-4-...`: workflow ingestion (runs/jobs/steps,
per-attempt), workflow-run backfill, build-log storage to MinIO, inferred
deployments, the event contract in `contracts/`, and — the headline — the
transactional-outbox relay that publishes to RabbitMQ. Verified end-to-end against
the live stack: a signed `workflow_run` webhook produces `workflow_runs` /
`workflow_jobs` / `workflow_steps` rows, an inferred `deployments` row, two
`event_outbox` rows written in the ingest transaction, and the relay publishes both
to the `buildlens.events` topic exchange with publisher confirms — a bound test
queue received both messages, each carrying the envelope with its `message_id`
idempotency key. A duplicate run emitted no new events (idempotent emission). The
untested-at-runtime paths are the ones needing real GitHub: live Actions backfill
and webhook-triggered log capture (log capture correctly *skips* when no member
token is available). **Phases 5-8 do not exist yet:** no Java, Python, or frontend.

Phase list and the reasoning behind each Phase 1 decision: `docs/phases.md`.

---

## How to work here

**One phase at a time.** Each phase ships something runnable and is reviewed by the
owner before the next starts. Do not scaffold later phases early. An empty
directory with a README is the correct state for work that has not been approved.

**Explain design decisions as you go.** The owner wants to understand the system,
not receive finished code. Where the Rust/Java/Python boundary or an event contract
needs a judgement call, surface it rather than silently picking.

**Keep changes scoped to the current phase.** Phases 1–3 are merged. Phase 4 is in
review. Work each phase on its own branch (`feat/phase-N-...`) and open it for
review before it merges — the owner reviews, with Claude, before the next phase
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
enforced by grants in `000010_grants.up.sql`, not by convention. Analytics
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
  src/routes/       auth/tenancy/tokens plus repository discovery and tracking
infra/migrations/ 11 migrations, 27 tables. The source of truth for the schema.
infra/postgres/   init/01-roles.sh: creates the 3 service roles (passwords can't
                  live in a migration, so role creation cannot either)
infra/minio/      bootstrap.sh: creates the buckets
contracts/        the event envelope, per-event examples, topology, versioning
analytics/        empty. Phase 5.
ai-worker/        empty. Phase 6.
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

Gate on `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
`cargo test`; all pass today.

---

## Phase 2: Auth & GitHub OAuth (Rust). Delivered.

The OAuth flow, opaque Redis sessions (httpOnly cookie, revocable — not a JWT),
AES-GCM-encrypted GitHub tokens, hashed API tokens with a clear lookup prefix, the
`owner|admin|member|viewer` org model with a personal org per user, and audit
logging are all in. The API surface is in `README.md`; the load-bearing decisions
are summarised under "What exists" and enforced by the invariants above. Config
added: `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `GITHUB_REDIRECT_URI`,
`TOKEN_ENCRYPTION_KEY`, `SESSION_TTL_SECONDS`.

---

## Phase 3: Repository sync & webhooks (Rust). Implemented, pending review.

Phase 3 is where facts start flowing out of GitHub. Phase 2 left us a logged-in
user with an encrypted OAuth token; Phase 3 uses it to populate the repository side
of the schema — `repositories`, `repository_sync_state`, `branches`, `commits`,
`pull_requests` — and stands up the webhook receiver that keeps them current. It is
**Rust-only**. No Java, no Python, no RabbitMQ yet.

### Decisions locked by the owner

**GitHub is called as the user, with their decrypted OAuth token** — not a GitHub
App installation token. The Phase 2 scopes (`read:user user:email repo read:org`)
already cover reading repos and managing repo hooks. A GitHub App would give higher
rate limits and per-install webhooks, at the cost of a much larger setup; for a
portfolio project the user-token path is the pragmatic call. **Boundary flag:** if
you ever want org-wide install without each user having admin, that is the App
route — decide now, because it changes the webhook story below.

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
is dropped instead of double-counting. The table doubles as a replay log — the fix
for a processing bug is to correct the code and re-drive the stored payloads, not to
beg GitHub to resend.

**Receive fast, process off the hot path.** The handler's job is verify → persist
the raw delivery → return `2xx` quickly so GitHub doesn't time out and retry. Turning
a delivery into `branches` / `commits` / `pull_requests` rows happens separately;
`webhook_deliveries_pending_idx` is literally "the work queue for the webhook
processor". A single-process background task draining that queue is fine for Phase 3.

**Boundary flag — the outbox stays empty in Phase 3.** The gateway *has* the grant
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
- **Webhook receiver:** `POST /webhooks/github` — verify signature, dedupe on
  `github_delivery_id`, persist to `webhook_deliveries`, return `2xx`.
- **Webhook processor:** drain pending deliveries and apply `push`,
  `pull_request`, and `pull_request_review` events to branches, commits, PRs, and
  `first_review_at`. Resolve the soft `repository_id` from `github_repo_id`; a
  webhook for an untracked or not-yet-synced repo is recorded and ignored.
- **Audit + authorization.** Reads gated by org membership; tracking changes audited
  (`repository.tracking_enabled` / `.disabled`) with the existing `audit::write`.

### Config added

- `GITHUB_WEBHOOK_SECRET` — the HMAC secret for signature verification.
- `GITHUB_WEBHOOK_URL` — the public callback registered on tracked repositories.
- `GITHUB_API_BASE_URL` — defaults to `https://api.github.com` and can target a
  test server.

### Deliberately absent (do NOT build in Phase 3)

`workflow_runs` / `jobs` / `steps` ingestion and build-log storage to MinIO
(Phase 4). Deployment ingestion (depends on workflow runs — Phase 4+). RabbitMQ
publishing and the outbox relay (Phase 4). DORA / scoring / flaky-test analytics
(Phase 5, Java). Anything Python or frontend.

---

## Decisions recorded during Phase 3

**`pull_request_review` events are included.** Initial sync backfills the first
review timestamp and webhook processing keeps it current.

**Webhooks are registered automatically.** Tracking requires a BuildLens `admin`
and GitHub repository admin permission. Registration failures are returned to the
caller; the repository is not marked tracked when its hook is absent.

**Who parses JUnit XML into `test_results`?** (Phase 5 concern, noted here so it is
not forgotten.) Currently granted to the gateway, on the reasoning that parsing test
reports means downloading an artifact from GitHub and the gateway holds the
credentials. If Phase 5 would rather Java did it, that is a one-line change in
`000010_grants.up.sql`.

---

## Phase 4: Workflow ingestion, log storage & the event relay (Rust). Implemented, pending review.

Phase 4 turns builds into facts and stands up the message bus. Still **Rust-only** —
no Java or Python — but it defines the contract those services will consume.

### Decisions made in Phase 4

**Events are triggers, not the source of truth.** Postgres is. An event says "repo
X has a new completed run — recompute", carrying only routing + an idempotency key;
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
Real GitHub Deployment ingestion is additive and deferred — see `docs/phases.md`.

**Log storage is best-effort and off the hot path.** On a completed run the
processor spawns a task that downloads the run's log zip and stores it to MinIO;
any failure is logged and dropped, never blocking the facts that already committed.
The run log is stored as-is (zipped); unpacking is a consumer's job.

**The relay borrows a member's GitHub token for webhook-time log capture.** A
webhook has no session, so `github_api::repository_token` picks any org member with
a valid token. **Boundary flag:** a production system would use a GitHub App
installation token instead; this is a deliberate portfolio-scope simplification.

### Delivered scope

- **Workflow ingestion** — `workflow_run` and `workflow_job` webhooks and a
  resumable REST backfill (`workflows`, then `workflow_runs` with their jobs/steps)
  populate `workflows` / `workflow_runs` / `workflow_jobs` / `workflow_steps`,
  per-attempt, with soft head-commit / PR FK resolution.
- **The outbox relay** (`relay.rs`) — declares the `buildlens.events` topic
  exchange and a DLX, drains `event_outbox` with `FOR UPDATE SKIP LOCKED`,
  publishes with confirms, marks published only on ack, backs off and dead-ends
  after `MAX_ATTEMPTS`, and reconnects on connection loss.
- **Inferred deployments**, **build-log storage to MinIO**, and the **event
  contract** in `contracts/`.

### Config added

`RABBITMQ_URL`, and `S3_ENDPOINT` / `S3_REGION` / `S3_ACCESS_KEY` / `S3_SECRET_KEY`
/ `S3_LOGS_BUCKET`. All required at boot. `docker-compose.yml` maps them from the
existing RabbitMQ/MinIO vars; the gateway now depends on both being healthy.

### Deliberately absent (do NOT build in Phase 4)

DORA / scoring / flaky-test analytics (Phase 5, Java — it reads these facts and the
events). JUnit parsing into `test_results`. Real GitHub Deployment API ingestion.
Anything Python or frontend.
