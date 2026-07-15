FROM ghcr.io/astral-sh/uv:python3.13-bookworm-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy

WORKDIR /app
COPY pyproject.toml uv.lock ./
RUN uv sync --locked --no-dev --no-install-project

COPY README.md ./
COPY src src
RUN uv sync --locked --no-dev

RUN useradd --create-home --uid 10001 buildlens \
    && chown -R buildlens:buildlens /app
USER buildlens

EXPOSE 8082
CMD ["uv", "run", "--no-sync", "uvicorn", "buildlens_ai.main:app", "--host", "0.0.0.0", "--port", "8082"]
