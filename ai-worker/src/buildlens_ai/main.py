import logging
import secrets
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Annotated

from fastapi import FastAPI, Header, HTTPException, Request, status

from buildlens_ai.config import Settings
from buildlens_ai.errors import InvalidEventError
from buildlens_ai.models import (
    HealthResponse,
    ManualRetriggerRequest,
    ManualRetriggerResponse,
)
from buildlens_ai.runtime import Runtime

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s %(message)s",
)
LOG = logging.getLogger(__name__)


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    settings = Settings()  # type: ignore[call-arg]
    runtime = await Runtime.start(settings)
    app.state.runtime = runtime
    LOG.info("AI worker started")
    try:
        yield
    finally:
        await runtime.close()


app = FastAPI(title="BuildLens AI Worker", version="0.1.0", lifespan=lifespan)


@app.get("/health", response_model=HealthResponse)
async def health() -> HealthResponse:
    return HealthResponse(status="ok")


@app.get("/health/ready", response_model=HealthResponse)
async def readiness(request: Request) -> HealthResponse:
    runtime: Runtime = request.app.state.runtime
    dependencies: dict[str, str] = {}
    try:
        await runtime.database.ping()
        dependencies["postgres"] = "ok"
    except Exception:
        dependencies["postgres"] = "unavailable"
    dependencies["rabbitmq"] = "ok" if runtime.rabbit.ready else "unavailable"
    if any(value != "ok" for value in dependencies.values()):
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail=HealthResponse(status="degraded", dependencies=dependencies).model_dump(),
        )
    return HealthResponse(status="ok", dependencies=dependencies)


@app.post("/reports/retrigger", response_model=ManualRetriggerResponse, status_code=202)
async def retrigger(
    payload: ManualRetriggerRequest,
    request: Request,
    authorization: Annotated[str | None, Header()] = None,
) -> ManualRetriggerResponse:
    runtime: Runtime = request.app.state.runtime
    _authorize(authorization, runtime.settings)
    try:
        claim = await runtime.database.manual_retrigger(payload.workflow_run_id, payload.kind)
    except InvalidEventError as error:
        raise HTTPException(status_code=status.HTTP_409_CONFLICT, detail=str(error)) from error
    runtime.spawn(runtime.processor.process(claim))
    return ManualRetriggerResponse(report_id=claim.report_id, status="pending")


def _authorize(authorization: str | None, settings: Settings) -> None:
    expected = settings.ai_manual_trigger_token.get_secret_value()
    supplied = authorization.removeprefix("Bearer ") if authorization else ""
    if not supplied or not secrets.compare_digest(supplied, expected):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="valid internal bearer token required",
        )
