# BuildLens AI worker

Phase 6's Python service explains BuildLens facts and analytics. FastAPI provides
health and an authenticated manual retry endpoint; the durable RabbitMQ consumer
is the primary workload.

## Ownership and flow

The worker connects as `buildlens_ai`. It reads observed facts and Java-derived
metrics, reads bounded build-log archives from MinIO, and writes only
`ai_reports`, `ai_recommendations`, and its `ai_event_receipts` idempotency ledger.
Python never runs migrations; `infra/migrations` remains the schema owner.

`workflow_run.completed` and `deployment.recorded` remain thin triggers. The
worker validates their envelope, AMQP `message_id`, routing key, version, and
database aggregate, then claims the event and report before any model request.
Unknown versions, malformed/oversized messages, and bad scope are dead-lettered.

## Cost and data-egress policy

- Failed runs use `claude-opus-4-8`; summaries and digests use
  `claude-haiku-4-5`.
- Successful-build summaries and scheduled reports default to disabled.
- A Postgres advisory lock serializes the projected-cost check against a hard
  monthly cap (default `$10.00`). Unknown model prices fail closed.
- Only structured facts and bounded failing-log excerpts leave the box. Raw log
  archives are not sent, and the worker never fetches repository source files.
  Every outbound string is secret-redacted.
- The official Anthropic SDK supplies structured Pydantic output, token usage,
  prompt caching, and adaptive thinking for failure analysis. Reports retain the
  model, prompt version, token counts, cost, and latency. Billable failed attempts
  accumulate into the same totals, so retries cannot hide spend from the cap.

The event/report claims prevent ordinary RabbitMQ redelivery from producing a
second paid call. There is still an unavoidable distributed-systems window if the
process dies after Anthropic returns but before Postgres commits: no database
transaction can atomically include an external model response. Stale claims are
recoverable, so that rare case favors eventually producing the report over silent
loss.

## Develop with UV

```bash
uv sync --locked
uv run ruff format --check .
uv run ruff check .
uv run pytest
```

From the repository root, `make ai-check`, `make ai-test`, and `make ai-dev` run
the same UV-managed workflow. Configure the Phase 6 values shown in
`.env.example`; `ANTHROPIC_API_KEY` and `AI_MANUAL_TRIGGER_TOKEN` must not retain
their example placeholders.

Endpoints:

- `GET /health` - liveness only.
- `GET /health/ready` - Postgres and RabbitMQ readiness.
- `POST /reports/retrigger` - retry a failed per-run report using
  `Authorization: Bearer $AI_MANUAL_TRIGGER_TOKEN`.
