.DEFAULT_GOAL := help
SHELL := /bin/bash

COMPOSE := docker compose
MIGRATE := $(COMPOSE) run --rm migrator
MIGRATE_RW := docker run --rm -v $(CURDIR)/infra/migrations:/migrations migrate/migrate:v4.17.1

# Long-running services, which `--wait` blocks on until they report healthy.
INFRA   := postgres redis rabbitmq minio
# One-shot containers, which do a job and exit 0. They must NOT go in the
# `--wait` list: `--wait` treats any container that stops as a failed startup,
# even one that exited cleanly.
ONESHOT := minio-init migrator

-include .env
export

## ---------------------------------------------------------------------------
## Environment
## ---------------------------------------------------------------------------

.PHONY: env
env: ## Create .env from .env.example (does not overwrite an existing .env)
	@test -f .env && echo ".env already exists; leaving it alone" || (cp .env.example .env && echo "created .env")

## ---------------------------------------------------------------------------
## Stack
## ---------------------------------------------------------------------------

.PHONY: up
up: ## Start infrastructure (postgres, redis, rabbitmq, minio) and run migrations
	$(COMPOSE) up -d --wait $(INFRA)
	$(COMPOSE) run --rm minio-init
	$(COMPOSE) run --rm migrator
	@echo ""
	@echo "  postgres   localhost:$(POSTGRES_PORT)"
	@echo "  redis      localhost:$(REDIS_PORT)"
	@echo "  rabbitmq   localhost:$(RABBITMQ_MANAGEMENT_PORT) (management UI)"
	@echo "  minio      localhost:$(MINIO_CONSOLE_PORT) (console)"

.PHONY: up-all
up-all: ## Start infrastructure and the gateway container
	$(MAKE) up
	$(COMPOSE) --profile app up -d --build --wait --no-deps gateway

.PHONY: down
down: ## Stop everything (volumes preserved)
	$(COMPOSE) --profile app down

.PHONY: reset
reset: ## Stop everything and DELETE all data volumes
	$(COMPOSE) --profile app down -v

.PHONY: ps
ps: ## Show container status
	$(COMPOSE) ps

.PHONY: logs
logs: ## Tail logs (make logs SERVICE=gateway)
	$(COMPOSE) logs -f $(SERVICE)

## ---------------------------------------------------------------------------
## Database
## ---------------------------------------------------------------------------

.PHONY: migrate
migrate: ## Apply all pending migrations
	$(COMPOSE) run --rm migrator

.PHONY: migrate-down
migrate-down: ## Roll back the last migration
	$(MIGRATE) -path=/migrations -database=$(MIGRATOR_URL) down 1

.PHONY: migrate-status
migrate-status: ## Show the current schema version
	$(MIGRATE) -path=/migrations -database=$(MIGRATOR_URL) version

.PHONY: migrate-create
migrate-create: ## Create a new migration pair (make migrate-create NAME=add_foo)
	@test -n "$(NAME)" || (echo "usage: make migrate-create NAME=add_foo" && exit 1)
	$(MIGRATE_RW) -path=/migrations create -ext sql -dir /migrations -seq $(NAME)

.PHONY: psql
psql: ## Open a psql shell as the owner role
	$(COMPOSE) exec postgres psql -U $(POSTGRES_USER) -d $(POSTGRES_DB)

.PHONY: redis-cli
redis-cli: ## Open a redis-cli shell
	$(COMPOSE) exec redis redis-cli

MIGRATOR_URL := postgres://$(POSTGRES_USER):$(POSTGRES_PASSWORD)@postgres:5432/$(POSTGRES_DB)?sslmode=disable

## ---------------------------------------------------------------------------
## Gateway (Rust)
## ---------------------------------------------------------------------------

.PHONY: dev
dev: ## Run the gateway on the host against dockerised infrastructure
	cd gateway && cargo run

.PHONY: check
check: ## Type-check the gateway
	cd gateway && cargo check --all-targets

.PHONY: fmt
fmt: ## Format the gateway
	cd gateway && cargo fmt

.PHONY: lint
lint: ## Clippy, warnings as errors
	cd gateway && cargo clippy --all-targets -- -D warnings

.PHONY: test
test: ## Run gateway tests
	cd gateway && cargo test

.PHONY: help
help:
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-16s\033[0m %s\n", $$1, $$2}'
