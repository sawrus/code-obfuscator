---
name: troubleshooting
type: skill
description: Systematic backend debugging — reproduce, isolate root cause, implement fix with regression test.
related-rules:
  - architecture.md
  - testing.md
allowed-tools: Read, Write, Edit, Bash, Grep
---

# Troubleshooting Skill

> **Expertise:** Systematic debugging, log analysis, query profiling, memory/CPU profiling, regression tests.

## Debugging Framework (RRCA)

```
1. REPRODUCE — make the bug happen reliably before touching code
2. REDUCE    — find the smallest input that triggers the bug
3. CAUSE     — identify the specific code line/condition responsible
4. ADDRESS   — fix + regression test + verify fix doesn't reappear
```

Never fix what you can't reproduce. A guess-and-check fix is technical debt.

## Log Analysis Patterns

```bash
# Find all errors in last hour (structured logs with jq)
journalctl -u myapp --since "1 hour ago" | jq 'select(.level == "error")'

# Count errors by type
cat app.log | jq -r '.error_code' | sort | uniq -c | sort -rn | head -20

# Find slowest requests
cat access.log | jq 'select(.duration > 1000)' | jq -r '[.method, .path, .duration] | @csv'

# Trace a specific request by request_id
grep "request_id=req_abc123" app.log

# Find N+1 patterns: same query repeated many times in same request
grep "request_id=req_abc123" app.log | grep "db.query" | wc -l  # > 10 is suspicious
```

## Database Query Debugging

```sql
-- Show currently running queries (PostgreSQL)
SELECT pid, now() - pg_stat_activity.query_start AS duration, query, state
FROM pg_stat_activity
WHERE state != 'idle' AND query_start < now() - interval '5 seconds'
ORDER BY duration DESC;

-- Kill a blocking query
SELECT pg_terminate_backend(<pid>);

-- Find slow queries from pg_stat_statements
SELECT query, calls, mean_exec_time, total_exec_time
FROM pg_stat_statements
ORDER BY total_exec_time DESC LIMIT 10;

-- Check for table bloat (after many deletes/updates)
SELECT relname, n_dead_tup, n_live_tup,
       round(n_dead_tup * 100.0 / nullif(n_live_tup + n_dead_tup, 0), 2) AS dead_pct
FROM pg_stat_user_tables
WHERE n_dead_tup > 1000
ORDER BY dead_pct DESC;
```

## Memory Leak Detection (Python)

```python
# Detect growing memory with tracemalloc
import tracemalloc

tracemalloc.start()

# ... run suspected code ...

snapshot = tracemalloc.take_snapshot()
top_stats = snapshot.statistics("lineno")
for stat in top_stats[:10]:
    print(stat)  # shows file:line and allocated bytes

# Typical culprits:
# - Unbounded in-memory caches (dict that grows forever)
# - Event listeners not being removed
# - Circular references preventing GC
```

## HTTP Client Debugging

```python
import httpx
import logging

# Enable full request/response logging for debugging
logging.basicConfig(level=logging.DEBUG)
httpx_logger = logging.getLogger("httpx")
httpx_logger.setLevel(logging.DEBUG)

# Or manually log request details
async with httpx.AsyncClient() as client:
    response = await client.post(url, json=payload, timeout=30.0)
    print(f"Request: {response.request.method} {response.request.url}")
    print(f"Request body: {response.request.content}")
    print(f"Status: {response.status_code}")
    print(f"Response: {response.text[:500]}")
```

## Writing Regression Tests

```python
# Pattern: test must FAIL before the fix, PASS after
# Step 1: reproduce as a test
def test_order_creation_with_zero_quantity_raises_validation_error():
    """Regression: order with zero quantity was silently accepted (bug PROJ-198)"""
    with pytest.raises(ValidationError) as exc_info:
        CreateOrderRequest(items=[{"product_id": "prod_1", "quantity": 0}])

    assert "quantity" in str(exc_info.value)
    assert "greater than 0" in str(exc_info.value)

# Step 2: run test — it should FAIL on current code
# Step 3: implement fix
# Step 4: run test — it should now PASS
# Step 5: run full suite to confirm no regressions
```

## Common Root Cause Patterns

| Symptom | Likely cause | Investigation |
|---|---|---|
| Intermittent 500s after deploy | Exception in new code path | Check error logs at deploy time; git diff |
| Slow requests on specific endpoint | N+1 query or missing index | EXPLAIN ANALYZE on DB queries in that request |
| Memory grows over time | Cache without TTL or eviction | tracemalloc; check dict/list sizes in long-running processes |
| Random 429s from upstream | No retry-after respect; burst traffic | Check outbound request rate; add backoff |
| Test passes locally, fails in CI | Env var missing; timing issue | Compare env vars; add explicit waits |
| Occasional duplicate records | Missing idempotency key or DB unique constraint | Add unique constraint; add idempotency check |
