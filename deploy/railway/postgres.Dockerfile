# Postgres 18 for Railway, with the per-service role bootstrap baked in.
#
# Two reasons this is a custom image rather than Railway's managed Postgres:
#   1. The schema uses native uuidv7(), which only exists in Postgres 18.
#   2. The least-privilege login roles (buildlens_gateway/analytics/ai) are
#      created by infra/postgres/init/01-roles.sh, which the official image runs
#      from /docker-entrypoint-initdb.d on first boot. Railway cannot mount that
#      directory, so the script is copied into the image instead.
#
# Build context is the repo root; set the Railway service's Dockerfile path to
# deploy/railway/postgres.Dockerfile and attach a volume at /var/lib/postgresql.
# Provide POSTGRES_USER, POSTGRES_PASSWORD, POSTGRES_DB, and the three
# *_DB_PASSWORD variables the init script reads.
FROM postgres:18-alpine

COPY infra/postgres/init/ /docker-entrypoint-initdb.d/
