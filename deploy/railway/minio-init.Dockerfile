# One-off MinIO bucket bootstrap for Railway.
#
# The compose stack creates the log/artifact buckets with a mounted script;
# Railway cannot mount files, so the script is baked in here. Deploy as its own
# service that runs once and exits. Re-running is a no-op (buckets are created
# with --ignore-existing).
#
# Build context is the repo root; set the Railway service's Dockerfile path to
# deploy/railway/minio-init.Dockerfile. Provide MINIO_ROOT_USER,
# MINIO_ROOT_PASSWORD, MINIO_LOGS_BUCKET, MINIO_ARTIFACTS_BUCKET, and
# MINIO_ENDPOINT=http://minio.railway.internal:9000 (your MinIO service's
# internal address).
FROM minio/mc:latest

COPY infra/minio/bootstrap.sh /bootstrap.sh

ENTRYPOINT ["/bin/sh", "/bootstrap.sh"]
