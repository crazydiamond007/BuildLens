# Deploying BuildLens (single VM + Docker Compose)

This is the smallest real, publicly-reachable deployment: one Linux VM running the
whole stack under Docker Compose, with [Caddy](https://caddyserver.com/) as the
front door handling TLS automatically.

## Topology

```
                        ┌─────────────────────────────────────────────┐
  Internet ── :443 ──▶  │ Caddy (TLS)                                  │
  (browser +            │   /auth/*, /webhooks/*, /health*  ─▶ gateway │
   GitHub)              │   everything else                 ─▶ frontend│
                        │                                              │
                        │ gateway ─ analytics ─ ai-worker ─ frontend   │
                        │ postgres ─ redis ─ rabbitmq ─ minio          │
                        └─────────────────────────────────────────────┘
```

One domain serves both the app and the gateway, split by path. That shared origin
is deliberate: the gateway's session cookie is host-only, so putting the frontend
on a different host (e.g. `app.` vs `api.`) would log users out. Only `/auth/*`,
`/webhooks/*`, and `/health*` reach the gateway from outside; every other gateway
route is called by the frontend server-side over the internal network.

## Prerequisites

- **A VM** with a public IP. Ubuntu 22.04+ is fine. **≥ 4 GB RAM recommended** —
  the Rust gateway image is memory-hungry to build. On a 1–2 GB box either add
  swap (`fallocate -l 4G /swapfile && chmod 600 /swapfile && mkswap /swapfile &&
  swapon /swapfile`) or build images elsewhere and pull them.
- **Docker Engine + the Compose plugin** installed (`docker --version`,
  `docker compose version`).
- **A domain** you control, with a DNS **A record** pointing at the VM's IP.
- **Ports 80 and 443** open to the internet (Caddy needs 80 for the ACME
  challenge and 443 to serve).

## Steps

### 1. Get the code onto the VM

```bash
git clone https://github.com/crazydiamond007/BuildLens.git
cd BuildLens
```

### 2. Register a production GitHub App

Use a **separate** App from your local `buildlens-dev` one (keep dev pointing at
localhost). At <https://github.com/settings/apps/new>, set — substituting your
domain for `buildlens.example.com`:

| Field | Value |
|---|---|
| Callback URL | `https://buildlens.example.com/auth/github/callback` |
| Request user authorization (OAuth) during installation | ✅ |
| Setup URL (post-install) + Redirect on update | `https://buildlens.example.com/auth/github/setup` ✅ |
| Webhook URL | `https://buildlens.example.com/webhooks/github` |
| Webhook secret | a strong random string (put the same value in `.env.production`) |
| Repository permissions | Actions **Read**, Contents **Read**, Metadata **Read** |
| Account permissions | Email addresses **Read** |
| Subscribe to events | Workflow run, Workflow job, Installation, Installation repositories |

Then note the **App ID** and **slug**, and **generate a private key** (`.pem`).

### 3. Fill in `.env.production`

```bash
cp .env.production.example .env.production
```

Edit it and replace every `REPLACE_*` value and the example domain/email. Generate
secrets with the commands noted beside each line, e.g.:

```bash
openssl rand -base64 32          # TOKEN_ENCRYPTION_KEY (generate ONCE, never rotate)
openssl rand -hex 32             # GITHUB_WEBHOOK_SECRET, AI_MANUAL_TRIGGER_TOKEN
openssl rand -base64 24          # each DB / RabbitMQ / MinIO password
base64 -w0 your-app.private-key.pem   # GITHUB_APP_PRIVATE_KEY (one line)
```

`DOMAIN`, `FRONTEND_URL`, `GATEWAY_PUBLIC_URL`, `GITHUB_REDIRECT_URI`, and
`GITHUB_WEBHOOK_URL` must all use your real domain and match the App's URLs
exactly.

> The gateway refuses to boot in production if any secret is still a placeholder
> or the all-zero encryption key — a feature, not a snag.

### 4. Deploy

```bash
./deploy/deploy.sh
```

It validates config, builds the four images, runs migrations, and starts
everything, waiting until each service is healthy. First run takes a while
(compiling Rust/Java). Re-run it any time to ship a new checkout.

### 5. Verify

```bash
curl -fsS https://buildlens.example.com/health         # {"status":"ok"} via Caddy+TLS
```

Then in a browser: open the site → **Sign in with GitHub** (consent shows **no**
`repo` scope) → **Install** the App on a repo → you're redirected back and the
repo appears under **Settings → Repository tracking** → enable tracking → push a
commit that triggers a workflow → confirm the run and its logs show up. That last
step is the real proof the end-to-end pipeline works.

## Operations

```bash
# tail logs
docker compose --env-file .env.production -f deploy/docker-compose.prod.yml logs -f gateway

# redeploy after a git pull
git pull && ./deploy/deploy.sh

# stop (keeps data volumes)
docker compose --env-file .env.production -f deploy/docker-compose.prod.yml down

# back up Postgres (do this on a schedule — it is NOT automated yet)
docker compose --env-file .env.production -f deploy/docker-compose.prod.yml \
  exec -T postgres pg_dump -U buildlens buildlens | gzip > buildlens-$(date +%F).sql.gz
```

## Known limitations of this MVP

- **Backups are manual.** Postgres holds all analytics; MinIO holds captured
  logs. Schedule the `pg_dump` above (cron) and snapshot the `minio_data` volume
  or the whole VM.
- **No metrics/tracing.** Health and readiness endpoints exist; there is no
  Prometheus/Grafana wiring yet. `docker compose logs` is your window for now.
- **Single node, no HA.** Everything runs on one VM; data stores are containers
  on local volumes, not managed services. Fine for a first deployment; revisit
  (managed Postgres/Redis, object storage, replicas) as usage grows.
- **CI does not deploy.** Images build on the VM. A build/push-to-registry +
  remote-deploy workflow is a natural follow-up.
