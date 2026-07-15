# BuildLens analytics

Phase 5's Spring Boot consumer. It reads GitHub facts from the shared Postgres,
consumes thin triggers from RabbitMQ, and writes only the four derivation tables
granted to `buildlens_analytics`:

- `dora_metrics` — repository and organization rollups at daily, ISO-weekly, and
  monthly granularity.
- `build_scores` — one idempotently replaced score per workflow run.
- `repository_scores` — one retained daily snapshot for a trailing 30-day window.
- `flaky_tests` — pass/fail flips between run attempts on an unchanged commit.

The service never migrates. Hibernate runs `ddl-auto=validate`; Flyway and
Liquibase are disabled. Inserts omit primary keys so Postgres 18's `uuidv7()`
defaults remain the only ID generator.

## Run

From the repository root:

```bash
make up
make analytics-dev

curl localhost:8081/actuator/health
```

Or run the full application profile with `make up-all`. Configuration comes from
`ANALYTICS_DATABASE_URL`, `ANALYTICS_DB_PASSWORD`, and `RABBITMQ_URL`.

## Event behavior

The service owns durable queue `analytics.workflow_runs`, bound to
`buildlens.events` with `workflow_run.*` and `deployment.*`. Its queue dead-letters
to `buildlens.events.dlx`. Version 1 `workflow_run.completed` and
`deployment.recorded` envelopes are supported; malformed, mismatched, and unknown
versions are rejected without requeue. Transient database failures are requeued.
Messages are acknowledged only after all invoked database transactions return.

There is no processed-event ledger. Re-delivery uses the envelope ID as the
message identity and safely recomputes the same natural keys. Adding a durable
ledger would buy strict no-op duplicate handling, but every current side effect is
a deterministic upsert, so it would add a migration and grant without changing
the result.

## Metric definitions

- Deployment frequency is successful and failed production deployments per day
  in the calendar period.
- Lead time is the deployed commit's `authored_at` to the deployment's effective
  time. Resolution uses `deployments.commit_id`, then `(repository_id, sha)`; an
  unresolved commit still counts as a deployment but not as a lead-time sample.
- Change-failure rate is failed production deployments divided by all production
  deployments. MTTR measures each failure to the next successful production
  deployment in the same repository, including in organization rollups.
- p50/p90 values use linear interpolation. `sample_size` is the deployment count,
  and a performance band is withheld below five samples.
- Build score weights are reliability 50%, duration 30%, and test result 20%.
- Repository score weights are reliability 35%, deployment velocity 25%, test
  quality 20%, and duration efficiency 20%. A missing signal receives a visible
  neutral baseline of 50 in the JSON breakdown.
- A flaky flip compares one collapsed outcome per test and workflow run, scoped
  to the same workflow and unchanged `head_sha`; matrix jobs or unrelated
  workflows cannot manufacture a flip. Flake rate uses only comparable retry
  transitions as its denominator.

The hourly scheduled pass rebuilds the last 90 days of DORA periods, current
flaky verdicts, recent build scores, and the current repository-score snapshot.
This also closes the intentional race between the workflow event and the
gateway's best-effort, post-commit JUnit artifact download.

## Test

```bash
mvn test
mvn -DskipTests package
```
