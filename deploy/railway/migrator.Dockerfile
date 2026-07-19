# One-off schema migrator for Railway.
#
# The migrator is the ONLY thing allowed to change the schema (see AGENTS.md), so
# it stays a distinct step rather than something the gateway does on boot. This
# image bakes the migration files into the `migrate` tool; deploy it as its own
# Railway service whose job is to run once and exit.
#
# Build context is the repo root. On Railway set the service's root directory to
# the repo root and its Dockerfile path to deploy/railway/migrator.Dockerfile.
#
# Set the service variable DATABASE_URL to the Postgres service's URL with
# sslmode=disable, e.g.  ${{postgres.DATABASE_URL}}?sslmode=disable
# Redeploy this service whenever migrations change; it applies only new ones.
FROM migrate/migrate:v4.17.1

COPY infra/migrations /migrations

# Shell form so $DATABASE_URL expands at runtime.
ENTRYPOINT ["/bin/sh", "-c", "migrate -path /migrations -database \"$DATABASE_URL\" up"]
