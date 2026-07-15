# BuildLens

[![CI](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml/badge.svg)](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml)
[![Phase](https://img.shields.io/badge/phase-5%20in%20review-d29922.svg)](#status)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-000000.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Java](https://img.shields.io/badge/Java-25-ED8B00.svg?logo=openjdk&logoColor=white)](https://openjdk.org)
[![Spring Boot](https://img.shields.io/badge/Spring%20Boot-4-6DB33F.svg?logo=springboot&logoColor=white)](https://spring.io/projects/spring-boot)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-18-4169E1.svg?logo=postgresql&logoColor=white)](#requires)
[![Docker](https://img.shields.io/badge/Docker-Compose-2496ED.svg?logo=docker&logoColor=white)](#quick-start)

DevOps analytics for GitHub Actions. Collects CI/CD data, computes DORA metrics
and build health, and explains failures.

An event-driven, polyglot backend: Rust owns ingestion, Java owns analysis,
Python owns generation, and they meet at a message bus and a shared Postgres.

```
Next.js ──► Rust gateway ──► RabbitMQ ──┬──► Java analytics ──┐
            (auth, GitHub,              │                     ├──► Postgres
             webhooks, sync)            └──► Python AI worker ┘
                    │
                    └──► Postgres · Redis · MinIO
```

## Status

**Phase 5: Java analytics, implemented and pending owner review.** The Spring Boot
consumer validates (but never migrates) the shared schema, owns a durable RabbitMQ
queue, and idempotently computes DORA rollups, build/repository scores, and flaky
tests into the four tables granted to `buildlens_analytics`. The gateway also now
downloads bounded workflow artifacts and parses JUnit XML into `test_results`,
keeping GitHub credentials on the Rust side of the boundary. Phases 1–4 are merged.

`AGENTS.md` is the handoff: current state, the invariants not to break, and the
delivered Phase 2 design. `docs/phases.md` is the decision log.

## Quick start

```bash
make env      # create .env from .env.example
# Fill in the GitHub OAuth and webhook values and replace TOKEN_ENCRYPTION_KEY
# before using anything beyond local development.
make up       # postgres, redis, rabbitmq, minio + run migrations
make dev      # gateway on the host, against the dockerised stack
make analytics-dev # analytics on the host (run in another terminal)

curl localhost:8080/health         # liveness: is the process alive?
curl localhost:8080/health/ready   # readiness: can it reach its dependencies?
curl localhost:8081/actuator/health # analytics + database/RabbitMQ health
```

`make help` lists the rest.

## Running the full pipeline end to end

The steps below take a fresh checkout all the way to a real GitHub Actions run
showing up as facts in Postgres, an event on RabbitMQ, and a log archive in
MinIO. Run them in order.

### 0. Prerequisites

- Docker and Docker Compose.
- Rust 1.94+ (the gateway runs on the host via `make dev`).
- Java 25+ and Maven 3.6.3+ (analytics runs on the host via `make analytics-dev`).
- Node.js (for the webhook tunnel in step 3), or any equivalent tunnel.
- A **GitHub OAuth App** (Settings → Developer settings → OAuth Apps). Set its
  callback URL to `http://localhost:8080/auth/github/callback`. Note the client
  ID and generate a client secret.
- A **webhook forwarding URL** for local development, e.g. a channel from
  [smee.io](https://smee.io). GitHub can't reach `localhost`, so it delivers to
  this public URL and the tunnel forwards to the gateway.

### 1. Configure `.env`

```bash
make env       # copies .env.example -> .env (does not overwrite an existing one)
```

Then fill in, in `.env`:

- `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` — from the OAuth App above.
- `GITHUB_REDIRECT_URI` — `http://localhost:8080/auth/github/callback`.
- `GITHUB_WEBHOOK_URL` — your public tunnel URL (the smee channel). This is what
  BuildLens registers on each tracked repository.
- `GITHUB_WEBHOOK_SECRET` — any string of 32+ characters. BuildLens signs and
  verifies deliveries with it.
- `TOKEN_ENCRYPTION_KEY` — 32 bytes, base64-encoded (`openssl rand -base64 32`).
  The all-zero default is for throwaway local use only.

The RabbitMQ and S3/MinIO values already point at the compose services and need
no changes for local development.

### 2. Start the infrastructure

```bash
make up         # postgres, redis, rabbitmq, minio, then runs all migrations
```

This also creates the `buildlens-logs` / `buildlens-artifacts` MinIO buckets and
the three per-service Postgres roles.

### 3. Start the webhook tunnel (leave running)

Forward your public webhook URL to the gateway's receiver. With smee:

```bash
npx smee-client --url "$GITHUB_WEBHOOK_URL" --target http://localhost:8080/webhooks/github
```

The `--target` path must be `/webhooks/github`. GitHub signs each delivery with
`GITHUB_WEBHOOK_SECRET`; the tunnel forwards the signature header unchanged, so
verification still passes.

### 4. Start the gateway (leave running)

```bash
make dev
```

Watch the log for `log store configured` and `outbox relay connected to
rabbitmq`, then confirm it is healthy:

```bash
curl localhost:8080/health         # liveness
curl localhost:8080/health/ready   # readiness: postgres + redis reachable
```

In another terminal, start analytics and verify its dependency-aware health:

```bash
make analytics-dev
curl localhost:8081/actuator/health
```

### 5. Log in with GitHub

Open **http://localhost:8080/auth/github/login** in a browser and approve. This
completes the OAuth round-trip, stores your GitHub token (encrypted), creates
your personal organization, and sets the `buildlens_session` cookie.

Find your organization id and copy the session cookie for the API calls below —
in the same browser, open **http://localhost:8080/me** for the org `id`, and copy
the `buildlens_session` cookie value from the browser dev tools.

### 6. Discover and track a repository

List what you can see (session-only):

```
http://localhost:8080/github/repositories
```

Then track one. This requires the BuildLens `admin` role (your personal org owner
qualifies) **and** GitHub repository admin permission, because BuildLens registers
the webhook for you. It also queues the initial backfill.

```bash
curl -X PUT \
  --cookie "buildlens_session=<COOKIE>" \
  http://localhost:8080/organizations/<ORG_ID>/github-repositories/<GITHUB_REPO_ID>/tracking
```

The backfill walks branches → commits → pull requests → workflows → workflow runs
(with their jobs and steps), checkpointing each page so a failed run resumes when
you repeat the call.

### 7. Trigger a run and watch it flow

Push a commit (or re-run a workflow) on the tracked repository. Then observe each
stage:

```bash
make psql   # then, in the psql shell:
SELECT github_run_id, status, conclusion, is_default_branch
  FROM workflow_runs ORDER BY created_at DESC LIMIT 5;
SELECT status, event_type FROM event_outbox        -- flips to 'published'
  ORDER BY created_at DESC LIMIT 5;
SELECT object_key, size_bytes FROM build_logs      -- run logs captured
  ORDER BY created_at DESC LIMIT 5;
SELECT overall_score, grade FROM repository_scores
  ORDER BY computed_at DESC LIMIT 5;
SELECT granularity, deployment_count, lead_time_p50_seconds
  FROM dora_metrics ORDER BY period_start DESC LIMIT 10;
```

To watch the messages on the bus, bind a throwaway queue to the events exchange
**before** the run (a topic exchange drops messages with no bound queue):

```bash
curl -s -u buildlens:buildlens_dev_password -H content-type:application/json \
  -XPUT  http://localhost:15672/api/queues/buildlens/observe -d '{"durable":true}'
curl -s -u buildlens:buildlens_dev_password -H content-type:application/json \
  -XPOST http://localhost:15672/api/bindings/buildlens/e/buildlens.events/q/observe \
  -d '{"routing_key":"#"}'
```

Then, using `buildlens` / `buildlens_dev_password`:

- **RabbitMQ** — http://localhost:15672 → Queues → `observe` shows the
  `workflow_run.completed` / `deployment.recorded` messages arriving.
- **MinIO** — http://localhost:9001 → the `buildlens-logs` bucket holds the run
  log zips.

### 8. Shut down

```bash
make down       # stop everything, keep data volumes
make reset      # stop everything and DELETE all volumes
```

## Phase 2 API

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/auth/github/login` | Start GitHub OAuth |
| GET | `/auth/github/callback` | Verify OAuth state, connect the account, issue a session |
| GET | `/auth/logout` | Revoke the current Redis session |
| GET | `/me` | Current user and organization memberships |
| GET, POST | `/organizations` | List or create BuildLens workspaces |
| GET, POST | `/organizations/{id}/members` | List, add, or change members |
| DELETE | `/organizations/{id}/members/{user_id}` | Remove a member |
| GET, POST | `/api-tokens` | List or issue read-only API tokens |
| DELETE | `/api-tokens/{id}` | Revoke an API token |

Protected routes accept either the `buildlens_session` httpOnly cookie or
`Authorization: Bearer blq_...`. Account and membership mutations require the
session cookie.

## Phase 3 API

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/github/repositories` | Discover visible GitHub repositories and their tracking state |
| GET | `/organizations/{id}/repositories` | List repositories in a BuildLens organization |
| PUT | `/organizations/{id}/github-repositories/{github_repo_id}/tracking` | Register the webhook, enable tracking, and queue initial sync |
| DELETE | `/organizations/{id}/repositories/{repository_id}/tracking` | Remove the webhook and stop tracking |
| POST | `/webhooks/github` | Verify, deduplicate, and persist GitHub deliveries |

Tracking mutations require a session and at least the BuildLens `admin` role.
GitHub repository admin permission is also required because BuildLens registers
the webhook automatically. Initial synchronization checkpoints each GitHub page
in `repository_sync_state`; repeating the tracking `PUT` safely resumes a failed
sync. Repository discovery is session-only because it exposes repositories that
have not entered a BuildLens authorization boundary. Webhook reception responds
before a background task applies deliveries.

Set `GITHUB_WEBHOOK_URL` to a public HTTPS endpoint in real deployments. For local
development, use a webhook forwarding tunnel. `GITHUB_API_BASE_URL` defaults to
`https://api.github.com` and can point at a test server.

## Layout

| Path         | What                                                       |
| ------------ | ---------------------------------------------------------- |
| `gateway/`   | Rust + Axum. Auth, GitHub API, repository sync and webhooks. |
| `analytics/` | Java + Spring Boot. DORA, scoring, flaky-test analytics.    |
| `ai-worker/` | Python + FastAPI. Summaries, recommendations. *Phase 6.*    |
| `frontend/`  | Next.js dashboard. *Phase 7.*                              |
| `contracts/` | Shared RabbitMQ event envelope, per-event examples, topology. |
| `infra/`     | Migrations, compose config, service roles, bucket setup.    |

## Three things worth knowing before reading the code

**The schema is owned by `infra/migrations`, not by any service.** Three services
share one Postgres, and per-service migration tools would mean three tools racing
on one database with no single file that describes the schema. Instead: one
migrator container, and per-service Postgres roles whose grants make the
ownership boundary real. The gateway may write facts; analytics may write derived
metrics; the AI worker may write its reports; nobody may edit the audit log. See
`infra/migrations/000010_grants.up.sql`.

**Foreign keys across event streams are nullable on purpose.** GitHub webhooks
arrive out of order. A `workflow_run.completed` routinely lands before the
`push` that created its commit. So runs store `head_sha` as text and carry a
nullable `head_commit_id` resolved later. Strict FKs there would deadlock
ingestion against itself. Within a stream (job → run, step → job), ordering is
guaranteed and the FKs are strict.

**Events go through a transactional outbox.** The gateway writes the row and the
event in one Postgres transaction; a relay publishes to RabbitMQ afterwards. That
makes delivery at-least-once rather than maybe-once, so every consumer must be
idempotent. Duplicates are survivable; silent loss is not.

## Requires

Docker, Docker Compose, Rust 1.94+, Java 25+, and Maven 3.6.3+. Postgres 18 is
required because the schema uses native `uuidv7()`; the compose file pins it.
