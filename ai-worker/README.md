# AI worker (Python / FastAPI)

AI-generated build summaries and recommendations.

**Phase 6.** Consumes RabbitMQ events, writes `ai_reports` and
`ai_recommendations`, and only those tables.

No migrations here. The schema is owned by `infra/migrations`.
