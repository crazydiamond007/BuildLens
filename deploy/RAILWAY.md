# Deploying BuildLens on Railway

Railway runs each piece as its own service on a shared private network. This guide
stands up the whole stack: four app services, four data services, and two one-off
jobs (migrations, bucket creation).

## Topology

```
  Internet ──▶ frontend (PUBLIC, Next.js)
                 │  rewrites /auth/* and /webhooks/*  ─┐
                 │  (all other data calls, server-side)│
                 ▼                                      ▼
              (private network, *.railway.internal)  gateway ─┐
                                                              │
     analytics ── ai-worker ── gateway  all talk to:  postgres · redis · rabbitmq · minio
```

**Only the frontend is public.** The gateway, workers, and data stores stay on
the private network. The browser reaches the gateway solely through the
frontend's `/auth/*` and `/webhooks/*` rewrites (see `frontend/next.config.ts`) —
this is what keeps everything on one origin so the gateway's host-only session
cookie works.

## Three things that will bite you if you skip them

1. **Postgres must be 18.** The schema uses native `uuidv7()`. Railway's managed
   Postgres has been 16, which fails migration `000001`. Deploy the custom
   `deploy/railway/postgres.Dockerfile` (Postgres 18 + the role bootstrap) instead
   of the one-click plugin.
2. **Railway's private network is IPv6-only.** A service reached internally must
   listen on `::`, not just `0.0.0.0`. Two services are reached this way:
   - **gateway** — set `GATEWAY_HOST=[::]`
   - **frontend** — set `HOSTNAME=::` (overrides the Dockerfile's `0.0.0.0`)
3. **Init scripts don't run on their own.** The Postgres role setup and the MinIO
   bucket creation run from mounted files in compose; Railway can't mount files,
   so they're baked into the custom images under `deploy/railway/` and run as a
   custom Postgres image + a one-off `minio-init` service.

## Services to create

Create one Railway project, then add these services. "Root dir" is the service's
root directory setting; images built from a Dockerfile use the paths below.

| Service | Source | Public? | Volume |
|---|---|---|---|
| `postgres` | Dockerfile `deploy/railway/postgres.Dockerfile` (root dir = repo root) | private | `/var/lib/postgresql` |
| `redis` | image `redis:7-alpine`, cmd `redis-server --appendonly yes` | private | `/data` |
| `rabbitmq` | image `rabbitmq:3.13-management-alpine` | private | `/var/lib/rabbitmq` |
| `minio` | image `minio/minio`, cmd `server /data` | private | `/data` |
| `migrator` | Dockerfile `deploy/railway/migrator.Dockerfile` (root dir = repo root) | private, one-off | — |
| `minio-init` | Dockerfile `deploy/railway/minio-init.Dockerfile` (root dir = repo root) | private, one-off | — |
| `gateway` | root dir `gateway/` (uses `gateway/railway.json`) | private | — |
| `analytics` | root dir `analytics/` | private | — |
| `ai-worker` | root dir `ai-worker/` | private | — |
| `frontend` | root dir `frontend/` | **PUBLIC** (generate a domain) | — |

## Environment variables

Generate the secrets exactly as in `.env.production.example`
(`openssl rand -base64 32` for the encryption key, etc.). Set them per service.
Cross-service references use Railway's `${{service.VAR}}` syntax. Internal
addresses are `<service>.railway.internal`.

Let **`APP_URL`** be your frontend's public URL, e.g.
`https://buildlens-production.up.railway.app` (or a custom domain).

**postgres**
```
POSTGRES_USER=buildlens
POSTGRES_PASSWORD=<owner password>
POSTGRES_DB=buildlens
GATEWAY_DB_PASSWORD=<gw password>
ANALYTICS_DB_PASSWORD=<an password>
AI_DB_PASSWORD=<ai password>
```

**redis / rabbitmq / minio** — set the credentials you'll reference elsewhere
(`RABBITMQ_DEFAULT_USER/PASS/VHOST`, `MINIO_ROOT_USER/MINIO_ROOT_PASSWORD`).

**migrator** (one-off)
```
DATABASE_URL=postgres://${{postgres.POSTGRES_USER}}:${{postgres.POSTGRES_PASSWORD}}@postgres.railway.internal:5432/${{postgres.POSTGRES_DB}}?sslmode=disable
```

**minio-init** (one-off)
```
MINIO_ENDPOINT=http://minio.railway.internal:9000
MINIO_ROOT_USER=${{minio.MINIO_ROOT_USER}}
MINIO_ROOT_PASSWORD=${{minio.MINIO_ROOT_PASSWORD}}
MINIO_LOGS_BUCKET=buildlens-logs
MINIO_ARTIFACTS_BUCKET=buildlens-artifacts
```

**gateway** (the URL wiring is the important part — all four public URLs are `APP_URL`)
```
ENVIRONMENT=production
GATEWAY_HOST=[::]
GATEWAY_PORT=8080
RUST_LOG=info,buildlens_gateway=info,sqlx=warn
DATABASE_URL=postgres://buildlens_gateway:${{postgres.GATEWAY_DB_PASSWORD}}@postgres.railway.internal:5432/${{postgres.POSTGRES_DB}}
DATABASE_MAX_CONNECTIONS=20
DATABASE_CONNECT_TIMEOUT_SECONDS=5
REDIS_URL=redis://redis.railway.internal:6379
RABBITMQ_URL=amqp://${{rabbitmq.RABBITMQ_DEFAULT_USER}}:${{rabbitmq.RABBITMQ_DEFAULT_PASS}}@rabbitmq.railway.internal:5672/${{rabbitmq.RABBITMQ_DEFAULT_VHOST}}
S3_ENDPOINT=http://minio.railway.internal:9000
S3_REGION=us-east-1
S3_ACCESS_KEY=${{minio.MINIO_ROOT_USER}}
S3_SECRET_KEY=${{minio.MINIO_ROOT_PASSWORD}}
S3_LOGS_BUCKET=buildlens-logs
GITHUB_API_BASE_URL=https://api.github.com
GITHUB_CLIENT_ID=<app client id>
GITHUB_CLIENT_SECRET=<app client secret>
GITHUB_APP_ID=<app id>
GITHUB_APP_SLUG=<app slug>
GITHUB_APP_PRIVATE_KEY=<base64 of the .pem>
GITHUB_WEBHOOK_SECRET=<32+ random chars>
TOKEN_ENCRYPTION_KEY=<base64 of 32 random bytes; set ONCE, never rotate>
SESSION_TTL_SECONDS=604800
FRONTEND_URL=<APP_URL>
GITHUB_REDIRECT_URI=<APP_URL>/auth/github/callback
GITHUB_WEBHOOK_URL=<APP_URL>/webhooks/github
```

**analytics**
```
ANALYTICS_PORT=8081
ANALYTICS_DATABASE_URL=jdbc:postgresql://postgres.railway.internal:5432/${{postgres.POSTGRES_DB}}
ANALYTICS_DB_PASSWORD=${{postgres.ANALYTICS_DB_PASSWORD}}
RABBITMQ_URL=amqp://${{rabbitmq.RABBITMQ_DEFAULT_USER}}:${{rabbitmq.RABBITMQ_DEFAULT_PASS}}@rabbitmq.railway.internal:5672/${{rabbitmq.RABBITMQ_DEFAULT_VHOST}}
```

**ai-worker**
```
AI_PORT=8082
AI_DATABASE_URL=postgresql://buildlens_ai:${{postgres.AI_DB_PASSWORD}}@postgres.railway.internal:5432/${{postgres.POSTGRES_DB}}
RABBITMQ_URL=amqp://${{rabbitmq.RABBITMQ_DEFAULT_USER}}:${{rabbitmq.RABBITMQ_DEFAULT_PASS}}@rabbitmq.railway.internal:5672/${{rabbitmq.RABBITMQ_DEFAULT_VHOST}}
ANTHROPIC_API_KEY=<key>
AI_MANUAL_TRIGGER_TOKEN=<32+ random chars>
AI_FAILURE_MODEL=claude-opus-4-8
AI_SUMMARY_MODEL=claude-haiku-4-5
AI_MONTHLY_COST_CAP_USD=25.00
AI_SUCCESS_SUMMARIES_ENABLED=false
AI_SCHEDULED_REPORTS_ENABLED=false
S3_ENDPOINT=http://minio.railway.internal:9000
S3_REGION=us-east-1
S3_ACCESS_KEY=${{minio.MINIO_ROOT_USER}}
S3_SECRET_KEY=${{minio.MINIO_ROOT_PASSWORD}}
S3_LOGS_BUCKET=buildlens-logs
```

**frontend**
```
HOSTNAME=::
GATEWAY_INTERNAL_URL=http://gateway.railway.internal:8080
GATEWAY_PUBLIC_URL=<APP_URL>
FRONTEND_URL=<APP_URL>
```
> `GATEWAY_INTERNAL_URL` is read when the `/auth/*` and `/webhooks/*` rewrites are
> compiled, i.e. at **build** time — it is baked into the build. Railway exposes
> service variables to the build, so setting it on the service is enough; just be
> sure it's set before the frontend builds (redeploy if you add it later).

## GitHub App

Register a production App (keep `buildlens-dev` for local) with these URLs, all on
`APP_URL`:

- Callback URL: `<APP_URL>/auth/github/callback`
- Setup URL (+ Redirect on update): `<APP_URL>/auth/github/setup`
- Webhook URL: `<APP_URL>/webhooks/github`
- Permissions: Actions/Contents/Metadata **Read**, Account Email addresses **Read**
- Events: Workflow run, Workflow job, Installation, Installation repositories

## Deploy order

1. Deploy **postgres**, **redis**, **rabbitmq**, **minio** — wait until healthy.
2. Run **migrator** once (it applies the schema, then exits). Re-run it on any
   future migration.
3. Run **minio-init** once (creates the buckets).
4. Deploy **gateway**, **analytics**, **ai-worker**.
5. Deploy **frontend** and generate its public domain — that domain is `APP_URL`.
   If you set `APP_URL` before knowing the domain, generate it first, then fill
   the URL vars and redeploy the gateway + frontend.

## Verify

```
curl -fsS <APP_URL>/health          # proxied to the gateway → {"status":"ok"}
```

Then in a browser: open `APP_URL` → **Sign in with GitHub** (no `repo` scope) →
**Install** on a repo → land back on the app → the repo shows under
**Settings → Repository tracking** → enable tracking → push a commit that triggers
a workflow → confirm the run and its logs appear. That last step proves the whole
pipeline end-to-end.

## Notes & caveats

- **Worker healthchecks are off.** `analytics` and `ai-worker` are queue
  consumers; their `railway.json` omits an HTTP healthcheck so an IPv6-bind
  mismatch can't fail the deploy. Watch their logs to confirm they connected to
  RabbitMQ. If you want HTTP healthchecks, first make each bind `::` (Spring:
  `server.address=::`; uvicorn: host `::`).
- **Webhook body integrity.** The webhook is proxied through the frontend's
  rewrite. If GitHub deliveries ever fail signature verification, give the gateway
  its own public domain and point only the webhook URL at it directly — webhooks
  carry no cookie, so this doesn't affect login.
- **Backups & storage durability.** MinIO on a single Railway volume is not
  redundant; snapshot it (and `pg_dump` Postgres) on a schedule. Moving logs to an
  external S3/R2 later is a one-line `S3_ENDPOINT` change.
- **No metrics/tracing yet**, and CI doesn't deploy — Railway redeploys on push to
  the connected branch.
```
