---
name: observability
type: skill
description: Implement structured logging, distributed tracing, and metrics for production-ready backend services.
related-rules:
  - architecture.md
allowed-tools: Read, Write, Edit, Bash
---

# Observability Skill

> **Expertise:** Structured JSON logging, OpenTelemetry distributed tracing, Prometheus/RED metrics, alert design.

## Structured Logging

```python
import structlog
import logging

# Configure once at app startup
structlog.configure(
    processors=[
        structlog.contextvars.merge_contextvars,
        structlog.processors.add_log_level,
        structlog.processors.TimeStamper(fmt="iso"),
        structlog.processors.JSONRenderer(),   # machine-parseable
    ],
    logger_factory=structlog.PrintLoggerFactory(),
)

log = structlog.get_logger()

# Bind context per request (FastAPI middleware)
@app.middleware("http")
async def logging_middleware(request: Request, call_next):
    request_id = request.headers.get("X-Request-ID") or str(uuid4())
    structlog.contextvars.bind_contextvars(
        request_id=request_id,
        method=request.method,
        path=request.url.path,
    )
    response = await call_next(request)
    structlog.contextvars.unbind_contextvars("request_id", "method", "path")
    return response

# Usage in service/repository layer
log.info("order.created", order_id=order.id, user_id=user.id, amount=str(order.total))
log.warning("payment.retry", order_id=order_id, attempt=attempt, reason=str(error))
log.error("db.query_failed", table="orders", query_type="insert", exc_info=True)
```

### What NOT to log
```python
# ❌ Never log PII or secrets
log.info("user.login", email=user.email)      # PII — omit or hash
log.debug("auth.token", token=access_token)   # secret — never

# ✅ Log identifiers, not values
log.info("user.login", user_id=user.id)
log.info("auth.issued", token_jti=token_payload["jti"])
```

## Distributed Tracing (OpenTelemetry)

```python
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.trace.export import BatchSpanProcessor

# Setup (once at startup)
provider = TracerProvider()
provider.add_span_processor(BatchSpanProcessor(OTLPSpanExporter()))
trace.set_tracer_provider(provider)

tracer = trace.get_tracer("order-service")

# Instrument a service method
async def create_order(self, user_id: int, items: list) -> Order:
    with tracer.start_as_current_span("order.create") as span:
        span.set_attribute("user.id", user_id)
        span.set_attribute("items.count", len(items))

        try:
            order = await self.repo.create(user_id, items)
            span.set_attribute("order.id", order.id)
            return order
        except Exception as e:
            span.record_exception(e)
            span.set_status(trace.StatusCode.ERROR)
            raise
```

## Metrics (Prometheus / RED)

RED method: **R**ate · **E**rror rate · **D**uration for every service boundary.

```python
from prometheus_client import Counter, Histogram, start_http_server

# Define metrics at module level (not inside functions)
REQUEST_COUNT = Counter(
    "http_requests_total",
    "Total HTTP requests",
    ["method", "endpoint", "status_code"]
)

REQUEST_DURATION = Histogram(
    "http_request_duration_seconds",
    "HTTP request duration",
    ["method", "endpoint"],
    buckets=[0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
)

DB_QUERY_DURATION = Histogram(
    "db_query_duration_seconds",
    "DB query duration",
    ["operation", "table"],
    buckets=[0.001, 0.005, 0.01, 0.05, 0.1, 0.5]
)

# FastAPI middleware
@app.middleware("http")
async def metrics_middleware(request: Request, call_next):
    start = time.monotonic()
    response = await call_next(request)
    duration = time.monotonic() - start

    endpoint = request.url.path
    REQUEST_COUNT.labels(request.method, endpoint, response.status_code).inc()
    REQUEST_DURATION.labels(request.method, endpoint).observe(duration)
    return response
```

## Health Check Endpoint

```python
@router.get("/health", include_in_schema=False)
async def health(db: AsyncSession = Depends(get_db)):
    checks = {}
    try:
        await db.execute(text("SELECT 1"))
        checks["database"] = "ok"
    except Exception as e:
        checks["database"] = f"error: {e}"

    is_healthy = all(v == "ok" for v in checks.values())
    return JSONResponse(
        {"status": "ok" if is_healthy else "degraded", "checks": checks},
        status_code=200 if is_healthy else 503
    )
```

## Alerting Rules (Reference)

| Signal | Threshold | Severity |
|---|---|---|
| Error rate | > 1% over 5 min | P1 |
| p99 latency | > 2s over 5 min | P1 |
| p95 latency | > 500ms over 15 min | P2 |
| DB connection pool saturation | > 80% for 5 min | P2 |
| Health check failures | 2 consecutive | P1 |
