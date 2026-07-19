#!/bin/sh
# Creates the buckets the gateway will write build logs and artifacts to.
# Idempotent: --ignore-existing means re-running this is a no-op.

set -e

# Endpoint defaults to the compose service name; Railway (and any other network
# where MinIO is not simply "minio") overrides it with MINIO_ENDPOINT.
until mc alias set local "${MINIO_ENDPOINT:-http://minio:9000}" "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD" >/dev/null 2>&1; do
    echo "buildlens: waiting for minio..."
    sleep 1
done

mc mb --ignore-existing "local/${MINIO_LOGS_BUCKET}"
mc mb --ignore-existing "local/${MINIO_ARTIFACTS_BUCKET}"

# Build logs are large, write-once, and cheap to re-fetch from GitHub for 90
# days. Expiring them keeps the dev volume from growing without bound; the
# retention window is a real product decision to revisit in Phase 4.
mc ilm rule add --expire-days 90 "local/${MINIO_LOGS_BUCKET}" 2>/dev/null || true

echo "buildlens: buckets ready: ${MINIO_LOGS_BUCKET}, ${MINIO_ARTIFACTS_BUCKET}"
