#!/usr/bin/env bash
# BuildLens production deploy: build images, run migrations, start the stack.
#
# Run from the repo root on the VM:  ./deploy/deploy.sh
# Re-run any time to ship the current checkout (it rebuilds changed images and
# restarts in place). Data lives in named volumes and survives redeploys.
set -euo pipefail

cd "$(dirname "$0")/.."

ENV_FILE=".env.production"
COMPOSE_FILE="deploy/docker-compose.prod.yml"

if [[ ! -f "$ENV_FILE" ]]; then
	echo "error: $ENV_FILE not found." >&2
	echo "       cp .env.production.example $ENV_FILE  and fill in every REPLACE_* value." >&2
	exit 1
fi

# Fail fast on secrets left at their placeholder — the gateway would refuse to
# boot anyway, but catching it here is a clearer message.
if grep -qE '=(REPLACE_|buildlens\.example\.com|you@example\.com)' "$ENV_FILE"; then
	echo "error: $ENV_FILE still contains placeholder values:" >&2
	grep -nE '=(REPLACE_|buildlens\.example\.com|you@example\.com)' "$ENV_FILE" | sed 's/^/       /' >&2
	exit 1
fi

compose() {
	docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

echo "==> Validating compose configuration"
compose config --quiet

echo "==> Building application images (this can take a while on first run)"
compose build

echo "==> Starting the stack (migrations run automatically before the apps)"
compose up -d --wait

echo "==> Service status"
compose ps

DOMAIN="$(grep -E '^DOMAIN=' "$ENV_FILE" | cut -d= -f2)"
echo
echo "Deployed. Once DNS for ${DOMAIN} points at this host and Caddy has issued a"
echo "certificate (first request may take a few seconds), verify:"
echo "  curl -fsS https://${DOMAIN}/health        # gateway liveness via Caddy"
echo "  open   https://${DOMAIN}                  # the app"
echo
echo "Logs:   docker compose --env-file $ENV_FILE -f $COMPOSE_FILE logs -f gateway"
echo "Stop:   docker compose --env-file $ENV_FILE -f $COMPOSE_FILE down"
