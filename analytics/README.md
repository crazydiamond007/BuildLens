# Analytics service (Java / Spring Boot)

DORA metrics, flaky test detection, repository and build scoring.

**Phase 5.** Consumes RabbitMQ events, writes `dora_metrics`, `flaky_tests`,
`repository_scores`, and `build_scores`. It writes only those tables (see
`infra/migrations/000010_grants.up.sql`).

Runs with `spring.jpa.hibernate.ddl-auto=validate` and Flyway disabled: the
schema is owned by `infra/migrations`, and this service should refuse to boot if
its entities have drifted from it.
