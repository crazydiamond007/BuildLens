# BuildLens

[![CI](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml/badge.svg)](https://github.com/crazydiamond007/BuildLens/actions/workflows/ci.yml)
[![Phase](https://img.shields.io/badge/phase-7%20in%20review-d29922.svg)](#project-status)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-000000.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Java](https://img.shields.io/badge/Java-25-ED8B00.svg?logo=openjdk&logoColor=white)](https://openjdk.org)
[![Spring Boot](https://img.shields.io/badge/Spring%20Boot-4-6DB33F.svg?logo=springboot&logoColor=white)](https://spring.io/projects/spring-boot)
[![Python](https://img.shields.io/badge/Python-3.13-3776AB.svg?logo=python&logoColor=white)](https://www.python.org)
[![Node.js](https://img.shields.io/badge/Node.js-24-339933.svg?logo=nodedotjs&logoColor=white)](https://nodejs.org)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-18-4169E1.svg?logo=postgresql&logoColor=white)](#requirements)
[![Docker](https://img.shields.io/badge/Docker-Compose-2496ED.svg?logo=docker&logoColor=white)](#quick-start)

**BuildLens is an open-source DevOps analytics platform for GitHub Actions.** It
collects your CI/CD data, computes DORA metrics and build-health scores, detects
flaky tests, and uses an LLM to explain *why* builds fail - with evidence, not
guesses.

It is a polyglot, event-driven backend built to be read as much as run: **Rust**
ingests, **Java** analyses, **Python** generates, and they meet at a message bus
and a shared Postgres. A Next.js dashboard presents those results through the
authenticated gateway.

```
                          ┌──────────────► Java analytics ─────┐
Next.js  ──►  Rust gateway ──► RabbitMQ ──┤   (DORA, scores)    ├──►  Postgres
(dashboard)  (auth, GitHub,               └──► Python AI worker ┘
              webhooks, sync,                   (failure reports)
              log capture)  │
                            └──────────────►  Postgres · Redis · MinIO
```

---

## Table of contents

- [What BuildLens does](#what-buildlens-does)
- [Architecture](#architecture)
- [Project status](#project-status)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Running the full pipeline end to end](#running-the-full-pipeline-end-to-end)
- [Using the API](#using-the-api)
- [How it works - design decisions](#how-it-works--design-decisions)
- [The event contract](#the-event-contract)
- [Repository layout](#repository-layout)
- [Make commands](#make-commands)
- [Contributing](#contributing)
- [How to help / roadmap](#how-to-help--roadmap)
- [License](#license)

---

## What BuildLens does

Point BuildLens at a GitHub repository and it will:

- **Ingest the facts** - repositories, branches, commits, pull requests, workflow
  runs/jobs/steps (one row per re-run attempt), and the run's logs, kept current
  by signed GitHub webhooks and a resumable backfill.
- **Compute DORA metrics** - deployment frequency, lead time for changes, change
  failure rate, and MTTR, as p50/p90 percentiles (not misleading averages), per
  repository and per organization, at daily / weekly / monthly granularity.
- **Score build health** - a per-run build score and a trailing-window repository
  score, each with an inspectable weight breakdown.
- **Detect flaky tests** - outcome *flips* on an unchanged commit, which is what
  makes a test flaky rather than merely broken.
- **Explain failures with AI** - on a failed run, a grounded `failure_analysis`
  report and actionable recommendations, each citing the job/step ids, failing
  test keys, and log line ranges it was based on.

Everything is stored in one Postgres database. The dashboard reads narrow,
organization-scoped view models from the gateway and never connects to the
database directly.

---

## Architecture

Four services, one shared Postgres, one message bus. Each service is written in
the language best suited to its job and owns a clear slice of the data:

| Service | Language | Responsibility | Writes |
| ------- | -------- | -------------- | ------ |
| **gateway** | Rust + Axum | Auth, GitHub OAuth, repo sync, webhooks, workflow ingestion, log capture, event publishing | the observed **facts** |
| **analytics** | Java 25 + Spring Boot | DORA, build/repo scoring, flaky-test detection, scheduled recompute | the derived **numbers** |
| **ai-worker** | Python 3.13 + FastAPI | Grounded failure analysis, build summaries, recommendations | the AI **reports** |
| **frontend** | Next.js 16 + React 19 | Dashboard - talks only to the gateway, never to Postgres/RabbitMQ/S3 | - |

Supporting infrastructure: **PostgreSQL 18** (single source of truth), **Redis**
(opaque sessions), **RabbitMQ** (the event bus), and **MinIO** (S3-compatible
build-log storage).

**The data flow:** the gateway turns GitHub webhooks into rows in Postgres and,
in the same transaction, an entry in a transactional outbox. A relay publishes
those entries to RabbitMQ. Analytics and the AI worker consume them as *triggers*,
read the authoritative data from Postgres, and write their own derived tables.
Events carry just enough to route and de-duplicate; Postgres remains the source
of truth.

---

## Project status

Delivered in reviewed phases, each shipping something runnable:

| Phase | Scope | Status |
| ----- | ----- | ------ |
| 1 | Foundation: repo layout, compose stack, schema, gateway skeleton | ✅ done |
| 2 | Auth: GitHub OAuth, sessions, API tokens, orgs & membership | ✅ done |
| 3 | Repository sync: GitHub client, branch/commit/PR sync, webhooks | ✅ done |
| 4 | Workflow ingestion: runs/jobs/steps, log storage, event contract, outbox relay | ✅ done |
| 5 | Java analytics: DORA, flaky tests, scoring, scheduled recompute | ✅ done |
| 6 | Python AI worker: build summaries & failure recommendations | ✅ done |
| 7 | Next.js dashboard | 🔍 in review |
| 8 | Testing, hardening, deployment | ⏳ planned |

`docs/phases.md` is the decision log - the *why* behind each phase's load-bearing
choices. `AGENTS.md` is the contributor/agent handoff describing the current
state and the invariants not to break.

---

## Requirements

To run the full stack on the host you need:

- **Docker** and **Docker Compose** - for Postgres, Redis, RabbitMQ, MinIO, and
  the migrator.
- **Rust 1.94+** - the gateway (`make dev`).
- **Java 25+** and **Maven 3.6.3+** - analytics (`make analytics-dev`).
- **[uv](https://docs.astral.sh/uv/)** and **Python 3.13+** - the AI worker
  (`make ai-dev`).
- **Node.js 24+** and npm - the frontend (`make frontend-dev`) and webhook
  forwarding tunnel during local development.

PostgreSQL 18 is required because the schema uses native `uuidv7()`; the compose
file pins it, so you do not install it yourself.

You can also run **everything in containers** with `make up-all` and skip the
per-service host toolchains - the host tools are only needed for iterating on a
service locally.

---

## Quick start

```bash
make env      # create .env from .env.example
# Fill in the GitHub OAuth + webhook values and replace TOKEN_ENCRYPTION_KEY
# before using anything beyond throwaway local development (see step 1 below).
make up       # postgres, redis, rabbitmq, minio + run all migrations

make dev             # gateway   on http://localhost:8080  (terminal 1)
make analytics-dev   # analytics on http://localhost:8081  (terminal 2)
make ai-dev          # AI worker on http://localhost:8082  (terminal 3)
make frontend-dev    # frontend  on http://localhost:3000  (terminal 4)

# health checks
curl localhost:8080/health/ready     # gateway   (postgres + redis)
curl localhost:8081/actuator/health  # analytics (database + rabbitmq)
curl localhost:8082/health/ready     # AI worker (postgres + rabbitmq)
curl localhost:3000                  # frontend
```

`make help` lists every command. For the full walkthrough that ends in real
GitHub data flowing through the whole pipeline, keep reading.

---

## Running the full pipeline end to end

These steps take a fresh checkout all the way to a real GitHub Actions run showing
up as facts in Postgres, metrics computed by analytics, an AI report, an event on
RabbitMQ, and a log archive in MinIO. Run them in order.

### 0. Prerequisites

Install the [requirements](#requirements) above, then create two things on GitHub:

- A **GitHub OAuth App** (Settings → Developer settings → OAuth Apps). Set its
  callback URL to `http://localhost:8080/auth/github/callback`. Note the client
  ID and generate a client secret.
- A **webhook forwarding URL** for local development, e.g. a channel from
  [smee.io](https://smee.io). GitHub can't reach `localhost`, so it delivers to
  this public URL and a small tunnel forwards it to the gateway.

### 1. Configure `.env`

```bash
make env      # copies .env.example -> .env (never overwrites an existing one)
```

Fill in these values in `.env` (the RabbitMQ / MinIO / database values already
point at the compose services and need no changes for local dev):

| Variable | What to set it to |
| -------- | ----------------- |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | From the OAuth App above. |
| `GITHUB_REDIRECT_URI` | `http://localhost:8080/auth/github/callback`. |
| `FRONTEND_URL` | `http://localhost:3000`, the browser destination after login/logout. |
| `GITHUB_WEBHOOK_URL` | Your public tunnel URL (the smee channel). Registered on each tracked repo. |
| `GITHUB_WEBHOOK_SECRET` | Any 32+ character string. Used to sign & verify deliveries. |
| `TOKEN_ENCRYPTION_KEY` | 32 bytes, base64 (`openssl rand -base64 32`). The all-zero default is throwaway-only. |
| `ANTHROPIC_API_KEY` | An Anthropic API key, for the Phase 6 AI worker. |
| `AI_MANUAL_TRIGGER_TOKEN` | A random 32+ char token (`openssl rand -hex 32`) protecting the manual retry endpoint. |

### 2. Start the infrastructure

```bash
make up   # postgres, redis, rabbitmq, minio, then runs all migrations
```

This also creates the `buildlens-logs` / `buildlens-artifacts` MinIO buckets and
the three per-service Postgres login roles.

### 3. Start the webhook tunnel (leave running)

Forward your public webhook URL to the gateway's receiver. With smee:

```bash
npx smee-client --url "$GITHUB_WEBHOOK_URL" --target http://localhost:8080/webhooks/github
```

The `--target` path must be `/webhooks/github`. GitHub signs each delivery with
`GITHUB_WEBHOOK_SECRET`; the tunnel forwards the signature header unchanged, so
verification still passes.

### 4. Start the services (leave running)

```bash
make dev             # gateway    (terminal 1)
make analytics-dev   # analytics  (terminal 2)
make ai-dev          # AI worker  (terminal 3)
make frontend-dev    # frontend   (terminal 4)
```

Watch the gateway log for `log store configured` and `outbox relay connected to
rabbitmq`, then confirm each service is healthy:

```bash
curl localhost:8080/health/ready     # gateway:   postgres + redis reachable
curl localhost:8081/actuator/health  # analytics: database + rabbitmq
curl localhost:8082/health/ready     # AI worker: postgres + rabbitmq
```

> The AI worker only spends money on demand. Successful-build summaries and
> scheduled digests are **disabled by default**; only a failed run triggers a
> paid model call, and a monthly cost cap (default **$10**) bounds it.

### 5. Log in with GitHub

Open **http://localhost:8080/auth/github/login** in a browser and approve. This
completes the OAuth round-trip, stores your GitHub token (AES-GCM encrypted),
creates your personal organization, and sets the `buildlens_session` cookie.

To make the API calls below, you need your organization id and the session
cookie: open **http://localhost:8080/me** in the same browser to read the org
`id`, and copy the `buildlens_session` cookie value from your browser dev tools.

### 6. Discover and track a repository

List what you can see (session-only):

```
http://localhost:8080/github/repositories
```

Copy the numeric `id` of a repo, then track it. This requires the BuildLens
`admin` role (your personal-org owner qualifies) **and** GitHub repository admin
permission, because BuildLens registers the webhook for you. It also queues the
initial backfill.

```bash
curl -X PUT \
  --cookie "buildlens_session=<COOKIE>" \
  http://localhost:8080/organizations/<ORG_ID>/github-repositories/<GITHUB_REPO_ID>/tracking
```

The backfill walks branches → commits → pull requests → workflows → workflow runs
(with their jobs and steps), checkpointing each page so a failed run resumes when
you repeat the call.

### 7. Trigger a run and watch it flow

Push a commit (or re-run a workflow) on the tracked repository, then observe each
stage in the database:

```bash
make psql   # opens a psql shell; then:
```
```sql
-- facts (gateway)
SELECT github_run_id, status, conclusion, is_default_branch
  FROM workflow_runs ORDER BY created_at DESC LIMIT 5;
SELECT object_key, size_bytes FROM build_logs
  ORDER BY created_at DESC LIMIT 5;

-- the outbox drains to 'published' as the relay ships each event
SELECT status, event_type FROM event_outbox ORDER BY created_at DESC LIMIT 5;

-- derived numbers (analytics)
SELECT granularity, deployment_count, lead_time_p50_seconds
  FROM dora_metrics ORDER BY period_start DESC LIMIT 10;
SELECT overall_score, grade FROM repository_scores
  ORDER BY computed_at DESC LIMIT 5;

-- AI reports (ai-worker) - appears for failed runs
SELECT kind, status, model, cost_usd, latency_ms
  FROM ai_reports ORDER BY requested_at DESC LIMIT 5;
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

- **RabbitMQ** → http://localhost:15672 → Queues → `observe` shows the
  `workflow_run.completed` / `deployment.recorded` messages arriving.
- **MinIO** → http://localhost:9001 → the `buildlens-logs` bucket holds the run
  log archives.

### 8. Shut down

```bash
make down    # stop everything, keep data volumes
make reset   # stop everything and DELETE all data volumes
```

---

## Using the API

All protected gateway routes accept either the `buildlens_session` httpOnly
cookie or `Authorization: Bearer blq_...` (a read-only API token). Account and
membership mutations require the session cookie.

**Auth, users & organizations**

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/auth/github/login` | Start GitHub OAuth |
| GET | `/auth/github/callback` | Verify OAuth state, connect the account, issue a session, redirect to frontend (or back to sign-in with `?error=` when authorization is refused) |
| POST | `/auth/logout` | Revoke the current Redis session and clear the cookie. Idempotent, and POST so a cross-site link cannot trigger it |
| GET | `/me` | Current user and organization memberships |
| GET, POST | `/organizations` | List or create BuildLens workspaces |
| GET, POST | `/organizations/{id}/members` | List, add, or change members |
| DELETE | `/organizations/{id}/members/{user_id}` | Remove a member |
| GET, POST | `/api-tokens` | List or issue read-only API tokens |
| DELETE | `/api-tokens/{id}` | Revoke an API token |

**Repositories & webhooks**

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/github/repositories` | Discover visible GitHub repos and their tracking state |
| GET | `/organizations/{id}/repositories` | List repositories in an organization |
| PUT | `/organizations/{id}/github-repositories/{github_repo_id}/tracking` | Register the webhook, enable tracking, queue initial sync |
| DELETE | `/organizations/{id}/repositories/{repository_id}/tracking` | Remove the webhook and stop tracking |
| POST | `/webhooks/github` | Verify, deduplicate, and persist GitHub deliveries |

**Dashboard reads**

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/organizations/{id}/dashboard` | Organization DORA, repository scores, runs, flaky tests, reports, and recommendations |
| GET | `/organizations/{id}/repositories/{repository_id}/insights` | Repository score history, DORA, runs, tests, and AI output |
| GET | `/organizations/{id}/runs/{run_id}` | Workflow run, jobs, steps, tests, log metadata, report, and recommendations |

All dashboard reads accept the existing session or read-only API token and require
at least the `viewer` organization role.

**Service health & internals**

| Method | Path | Service | Purpose |
| ------ | ---- | ------- | ------- |
| GET | `/health`, `/health/ready` | gateway (8080) | liveness / readiness (postgres + redis) |
| GET | `/actuator/health` | analytics (8081) | health incl. database + RabbitMQ |
| GET | `/health`, `/health/ready` | ai-worker (8082) | liveness / readiness (postgres + rabbitmq) |
| POST | `/reports/retrigger` | ai-worker (8082) | retry a failed per-run report; needs `Authorization: Bearer $AI_MANUAL_TRIGGER_TOKEN` |

The Next.js Server Components consume these gateway views. Direct Postgres access
remains useful for the end-to-end verification SQL in step 7, but is not part of
the browser architecture.

---

## How it works - design decisions

A few load-bearing choices explain most of the codebase. The full rationale for
each is in `docs/phases.md`.

**The schema is owned by `infra/migrations`, not by any service.** Three services
share one Postgres. Per-service migration tools would mean three tools racing on
one database with no single file that describes the schema. Instead: one migrator
container runs the SQL migrations to completion before any service starts, and
**per-service Postgres roles** make the ownership boundary real - the gateway may
write facts, analytics may write derived metrics, the AI worker may write its
reports, and *nobody* may edit the audit log. See
`infra/migrations/000010_grants.up.sql`. Java runs `ddl-auto=validate` with Flyway
disabled; Python never migrates.

**Facts vs. derivations.** The gateway owns everything observed from GitHub;
analytics owns every number it computes; the AI worker owns its reports. Everyone
reads everything, but writes are constrained by the grants above. This is why
derived tables can always be truncated and rebuilt from the facts.

**Events go through a transactional outbox.** The gateway writes the fact row and
an `event_outbox` row in one Postgres transaction; a relay publishes to RabbitMQ
afterwards and marks it published. Delivery is therefore **at-least-once** rather
than maybe-once - a crash can duplicate a message but never lose it - so every
consumer is idempotent. Duplicates are survivable; silent loss is not.

**Foreign keys across event streams are nullable on purpose.** GitHub webhooks
arrive out of order and at-least-once. A `workflow_run.completed` routinely lands
before the `push` that created its commit, so a run stores `head_sha` as text and
carries a nullable `head_commit_id` resolved later. Making those FKs strict would
deadlock ingestion against itself. Within a stream (job → run, step → job),
GitHub guarantees ordering, so *those* FKs are strict.

**Every re-run attempt is its own `workflow_runs` row**, keyed on
`(github_run_id, run_attempt)`. Collapsing attempts would destroy exactly the
signal flaky-test detection depends on: same commit, same workflow, failed, then
passed on retry.

**AI spend is bounded and grounded.** Failed runs use `claude-opus-4-8`;
summaries and digests use `claude-haiku-4-5`. A Postgres advisory lock serializes
a projected-cost check against a hard monthly cap (default `$10`, fail-closed on
unknown prices). Only structured facts and bounded, **secret-redacted** failing-log
excerpts leave the box - never raw log archives or repository source. Reports
retain the model, prompt version, token counts, cost, and latency, so a duplicate
delivery cannot hide spend and a prompt change stays attributable.

**Metric definitions** (analytics): DORA percentiles use linear interpolation over
a period's deployments; a performance band is withheld below five samples. Lead
time is the deployed commit's `authored_at` to the deployment's effective time.
Build score = reliability 50% / duration 30% / tests 20%; repository score =
reliability 35% / velocity 25% / quality 20% / efficiency 20%, with a visible
neutral 50 baseline for missing signals. A flaky "flip" compares one collapsed
outcome per test and run, scoped to the same workflow and unchanged `head_sha`.

---

## The event contract

The RabbitMQ event schema shared by the gateway (Rust, producer) and the
analytics (Java) and AI (Python) consumers. Example payloads live in
[`contracts/`](contracts/).

### The one idea

**Events are triggers, not the source of truth.** Postgres is. An event says
"this repository has a new completed run - go recompute", and carries just enough
to route and de-duplicate it. The consumer reads Postgres for the detail. That
keeps the wire format small and stable while the schema underneath it evolves, and
it means a consumer that missed an event can always recover the full picture from
the database.

### The envelope

Every event is a JSON object with this envelope; event-specific fields live under
`data`:

```json
{
  "id": "0191...-uuid-v7",
  "type": "workflow_run.completed",
  "version": 1,
  "occurred_at": "2026-07-15T12:00:00Z",
  "aggregate": { "type": "workflow_run", "id": "0191...-uuid" },
  "organization_id": "0191...-uuid",
  "repository_id": "0191...-uuid",
  "data": { }
}
```

- `id` - a UUIDv7, unique per event, and also the AMQP `message_id`. **This is the
  idempotency key**: a consumer that already processed an `id` must treat a second
  delivery as a no-op.
- `type` - the event type; equals the AMQP routing key today.
- `version` - the envelope schema version; bumped only on a breaking change.
- `aggregate` - the domain object the event is about.
- `organization_id` / `repository_id` - denormalised onto every event so a
  consumer can scope work (and authorization) without a lookup.
- `occurred_at` - when the gateway recorded the fact, not when GitHub emitted it.

### Events

| type | aggregate | routing key | emitted when |
| ---- | --------- | ----------- | ------------ |
| `workflow_run.completed` | `workflow_run` | `workflow_run.completed` | a run first reaches `status = completed` |
| `deployment.recorded` | `deployment` | `deployment.recorded` | a successful default-branch run is inferred to be a production deployment |

Both are emitted only on the *transition* into their state, so a replayed webhook
does not re-fire them. The outbox still guarantees at-least-once delivery, so the
transition guard makes the common case exactly-once and the idempotency key covers
the rest. Reserved for later phases (consumers should not treat the list as
closed): `push.received`, `pull_request.merged`.

### Topology

- **Exchange** `buildlens.events` - `topic`, durable. The gateway declares it and
  publishes here; it declares nothing else consumer-facing.
- **Dead-letter exchange** `buildlens.events.dlx` - `topic`, durable. Declared by
  the gateway so it exists; consumers point their queues'
  `x-dead-letter-exchange` at it.
- **Queues and bindings are the consumer's responsibility.** A consumer declares
  its own durable queue and binds it to `buildlens.events` with the routing keys
  it wants. For example, analytics owns `analytics.workflow_runs` bound on
  `workflow_run.*` and `deployment.*`.

Messages are published `persistent` (delivery mode 2) with publisher confirms, so
a message the broker acked survives a broker restart. Consumers ack only after
they have durably processed (or dead-lettered) the message.

### Versioning & delivery

The producer is always deployed before its consumers, therefore:

- **Consumers MUST ignore unknown fields.** Adding a field is not breaking and
  does not bump `version`. Removing/repurposing a field or changing a type **is**
  breaking: it bumps `version`, and the producer emits both until consumers
  migrate. An unknown `version` is dead-lettered, not crashed on.
- Delivery is **at-least-once** by construction (the transactional outbox in
  `infra/migrations/000009_ingestion.up.sql`). Every consumer needs the
  idempotency key.

---

## Repository layout

```
gateway/          Rust + Axum - auth, GitHub API, sync, webhooks, ingestion, relay
analytics/        Java 25 + Spring Boot - DORA, scoring, flaky-test analytics
ai-worker/        Python 3.13 + FastAPI + uv - grounded summaries & recommendations
frontend/         Next.js 16 dashboard - talks only to the gateway
contracts/        Shared event payload examples (the contract itself is above)
infra/
  migrations/     The one source of truth for the schema (numbered .up/.down SQL)
  postgres/       init/01-roles.sh - creates the three per-service login roles
  minio/          bootstrap.sh - creates the object-storage buckets
docs/phases.md    The decision log - why each phase is built the way it is
AGENTS.md         Contributor/agent handoff: current state and invariants
Makefile          Every dev command (make help)
docker-compose.yml
```

---

## Make commands

```
make env             Create .env from .env.example
make up              Start infrastructure (postgres, redis, rabbitmq, minio) + migrations
make up-all          Start infrastructure and all application containers
make down            Stop everything (volumes preserved)
make reset           Stop everything and DELETE all data volumes
make ps / logs       Container status / tail logs (make logs SERVICE=gateway)

make dev             Run the gateway on the host
make analytics-dev   Run analytics on the host
make ai-dev          Run the AI worker on the host
make frontend-dev    Run the Next.js frontend on the host

make fmt / lint / test / check          Gateway: format, clippy (-D warnings), tests, type-check
make analytics-check / analytics-test   Analytics: compile (warnings denied), unit tests
make ai-check / ai-test                  AI worker: ruff format+lint, pytest
make frontend-check / frontend-build     Frontend: ESLint+TypeScript, production build

make migrate         Apply all pending migrations
make migrate-down    Roll back the last migration
make migrate-status  Show the current schema version
make migrate-create NAME=add_foo   Create a new migration pair

make psql            Open a psql shell as the owner role
make redis-cli       Open a redis-cli shell
```

---

## Contributing

Contributions are welcome - code, docs, and bug reports alike. The full guide is
in **[CONTRIBUTING.md](CONTRIBUTING.md)**; in short:

1. Fork and branch (`feat/<what>`), keep a change focused on one thing.
2. Read [`AGENTS.md`](AGENTS.md) for the invariants that must not break silently
   (schema ownership, per-service grants, nullable cross-stream FKs, the outbox,
   per-attempt run rows, UUIDv7 keys).
3. Add tests and make the checks pass on the service you touched - gateway:
   `make fmt lint test`; analytics: `make analytics-check analytics-test`;
   ai-worker: `make ai-check ai-test`; frontend: `make frontend-check
   frontend-build`.
4. Open a pull request against `main` describing what changed and why.

The database (`infra/migrations` only, add a grant when you add a table) and the
[event contract](#the-event-contract) have specific rules - see
[CONTRIBUTING.md](CONTRIBUTING.md). Found a vulnerability? Report it privately per
[SECURITY.md](SECURITY.md), not in a public issue.

---

## How to help / roadmap

The most useful contributions right now:

- **Phase 7 review.** Exercise the responsive dashboard against real tracked
  repositories and report any accessibility, data-shape, or browser issues.
- **Phase 8 - hardening.** Integration tests across services, deployment
  manifests, observability, table partitioning for the high-volume tables.
- **Real GitHub Deployment API ingestion** - deployments are currently *inferred*
  from successful default-branch runs; ingesting real Deployment objects is
  additive and welcome.
- **Docs, examples, and bug reports.** Try the walkthrough above and open an issue
  if a step doesn't work on your machine.

If you'd like to take something on, open an issue to discuss it first so we don't
duplicate work.

---

## License

BuildLens is released under the [MIT License](LICENSE) - free to use, modify, and
distribute, with attribution and without warranty.
