# BuildLens

[![CI](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml/badge.svg)](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml)
[![Phase](https://img.shields.io/badge/phase-3%20in%20review-d29922.svg)](#status)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-000000.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
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

**Phase 3: repository sync and webhooks, in review.** The gateway discovers a
user's GitHub repositories, tracks selected repositories, performs resumable
initial synchronization, and processes signed `push`, `pull_request`, and
`pull_request_review` webhooks. Phase 2 authentication remains the access-control
foundation. Phase 4 does not exist yet.

`AGENTS.md` is the handoff: current state, the invariants not to break, and the
delivered Phase 2 design. `docs/phases.md` is the decision log.

## Quick start

```bash
make env      # create .env from .env.example
# Fill in the GitHub OAuth and webhook values and replace TOKEN_ENCRYPTION_KEY
# before using anything beyond local development.
make up       # postgres, redis, rabbitmq, minio + run migrations
make dev      # gateway on the host, against the dockerised stack

curl localhost:8080/health         # liveness: is the process alive?
curl localhost:8080/health/ready   # readiness: can it reach its dependencies?
```

`make help` lists the rest.

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
| `analytics/` | Java + Spring Boot. DORA, scoring, flaky tests. *Phase 5.*  |
| `ai-worker/` | Python + FastAPI. Summaries, recommendations. *Phase 6.*    |
| `frontend/`  | Next.js dashboard. *Phase 7.*                              |
| `contracts/` | Shared RabbitMQ event schemas. *Phase 4.*                   |
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

Docker, Docker Compose, and Rust 1.94+ to run the gateway on the host. Postgres
18 (the schema uses native `uuidv7()`); the compose file pins it.
