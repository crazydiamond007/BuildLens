#!/bin/bash
# Creates the three per-service login roles.
#
# Runs exactly once, on an empty data volume, before the migrator. Roles are
# created here rather than in a migration because a migration file cannot hold a
# password without committing it to git.
#
# This script grants only the ability to connect and to READ. Write privileges
# are table-by-table and live in migration 000010. The schema decides who owns
# what, and it stays reviewable in one place.

set -euo pipefail

psql -v ON_ERROR_STOP=1 \
    -v gateway_password="$GATEWAY_DB_PASSWORD" \
    -v analytics_password="$ANALYTICS_DB_PASSWORD" \
    -v ai_password="$AI_DB_PASSWORD" \
    -v database_name="$POSTGRES_DB" \
    -v owner_role="$POSTGRES_USER" \
    --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-'EOSQL'
    CREATE ROLE buildlens_gateway   LOGIN PASSWORD :'gateway_password';
    CREATE ROLE buildlens_analytics LOGIN PASSWORD :'analytics_password';
    CREATE ROLE buildlens_ai        LOGIN PASSWORD :'ai_password';

    -- A group role, so "every service" grants can be written once.
    CREATE ROLE buildlens_services NOLOGIN;
    GRANT buildlens_services TO buildlens_gateway, buildlens_analytics, buildlens_ai;

    GRANT CONNECT ON DATABASE :"database_name" TO buildlens_services;
    GRANT USAGE   ON SCHEMA public          TO buildlens_services;

    -- No service may create or drop tables. Only the migrator (running as the
    -- owner) can, which is the whole point of the single-migrator decision.
    REVOKE CREATE ON SCHEMA public FROM PUBLIC;
    REVOKE CREATE ON SCHEMA public FROM buildlens_services;

    -- Everything the migrator creates from now on is readable by every service
    -- by default. Reads are shared; writes are not.
    ALTER DEFAULT PRIVILEGES FOR ROLE :"owner_role" IN SCHEMA public
        GRANT SELECT ON TABLES TO buildlens_services;
    ALTER DEFAULT PRIVILEGES FOR ROLE :"owner_role" IN SCHEMA public
        GRANT USAGE, SELECT ON SEQUENCES TO buildlens_services;
EOSQL

echo "buildlens: service roles created"
