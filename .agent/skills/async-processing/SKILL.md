---
name: async-processing
type: skill
description: Design and implement async task queues, message consumers, and background job patterns.
related-rules:
  - architecture.md
  - data_access.md
allowed-tools: Read, Write, Edit, Bash
---

# Async Processing Skill

> **Expertise:** Task queues (Celery, ARQ, Dramatiq), Kafka/NATS consumers, background jobs, retry strategies, idempotency, dead-letter queues.

## When to Use Async Processing

```
Use async when:
✅ Operation takes > 200ms (send email, resize image, call slow 3rd party)
✅ Work can be retried independently (payment webhook, notification)
✅ Decoupling producers from consumers is required
✅ Fan-out to multiple consumers needed

Keep synchronous:
❌ Response depends on the result (user sees outcome immediately)
❌ Must be transactional with the triggering DB write
```

## Task Queue (Celery + Redis)

```python
# tasks.py
from celery import Celery
from celery.utils.log import get_task_logger

app = Celery("myapp", broker="redis://localhost:6379/1", backend="redis://localhost:6379/2")
app.conf.update(
    task_serializer="json",
    result_expires=3600,
    task_acks_late=True,          # Ack after completion, not on receive
    task_reject_on_worker_lost=True,
    task_default_retry_delay=60,  # 1 min base delay
    task_max_retries=5,
)

logger = get_task_logger(__name__)

@app.task(bind=True, max_retries=5, default_retry_delay=30)
def send_order_confirmation(self, order_id: int) -> None:
    try:
        order = Order.objects.get(id=order_id)
        email_service.send_confirmation(order)
        logger.info("email.sent", extra={"order_id": order_id})
    except EmailServiceError as exc:
        # Exponential backoff: 30s, 60s, 120s, 240s, 480s
        delay = 30 * (2 ** self.request.retries)
        raise self.retry(exc=exc, countdown=delay)
    except Order.DoesNotExist:
        logger.error("order.not_found", extra={"order_id": order_id})
        # Don't retry — data issue, not transient
```

## Message Consumer (Kafka / aiokafka)

```python
from aiokafka import AIOKafkaConsumer
import asyncio, json

async def consume_order_events():
    consumer = AIOKafkaConsumer(
        "orders.events",
        bootstrap_servers="kafka:9092",
        group_id="notification-service",
        auto_offset_reset="earliest",
        enable_auto_commit=False,    # Manual commit — control exactly-once
        value_deserializer=lambda v: json.loads(v.decode()),
    )
    await consumer.start()
    try:
        async for msg in consumer:
            event = msg.value
            try:
                await handle_event(event)
                await consumer.commit()           # Only commit on success
            except TransientError as e:
                logger.warning("event.retry", event_type=event["type"], error=str(e))
                await asyncio.sleep(5)            # Back off, do NOT commit
            except PermanentError as e:
                logger.error("event.dead_letter", event=event, error=str(e))
                await dead_letter_queue.publish(event)
                await consumer.commit()           # Commit to move past poison message
    finally:
        await consumer.stop()

# Idempotency — always check before processing
async def handle_event(event: dict) -> None:
    event_id = event["event_id"]
    if await redis.exists(f"processed:{event_id}"):
        return  # Already handled — skip

    await process(event)
    await redis.setex(f"processed:{event_id}", 86400, "1")
```

## Background Jobs (ARQ — lightweight async)

```python
# worker.py
from arq import cron
from arq.connections import RedisSettings

async def cleanup_expired_sessions(ctx):
    deleted = await db.sessions.delete_expired()
    ctx["log"].info(f"Cleaned up {deleted} expired sessions")

async def startup(ctx):
    ctx["db"] = await create_db_pool()
    ctx["log"] = structlog.get_logger()

class WorkerSettings:
    functions = [cleanup_expired_sessions]
    cron_jobs = [cron(cleanup_expired_sessions, hour=3, minute=0)]  # 3 AM daily
    redis_settings = RedisSettings(host="redis")
    on_startup = startup
    max_jobs = 10
    job_timeout = 300   # 5 min max per job
```

## Retry Strategy Reference

| Error type | Strategy |
|---|---|
| Transient (network timeout, DB lock) | Exponential backoff, up to 5 retries |
| Rate limit (429) | Respect Retry-After header |
| Data validation (bad payload) | Dead-letter immediately — no retry |
| Downstream service down | Circuit breaker + retry queue |
| DB connection pool full | Short delay (5s) + 3 retries |

## Dead-Letter Queue Pattern

```python
async def send_to_dlq(original_message: dict, error: Exception, queue_name: str):
    dlq_message = {
        "original": original_message,
        "error": str(error),
        "error_type": type(error).__name__,
        "failed_at": datetime.utcnow().isoformat(),
        "source_queue": queue_name,
    }
    await redis.lpush("dlq", json.dumps(dlq_message))
    metrics.increment("tasks.dead_lettered", tags={"queue": queue_name})
```
