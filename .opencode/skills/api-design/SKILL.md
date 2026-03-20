---
name: api-design
type: skill
description: Design and implement REST APIs with consistent conventions, versioning, error contracts, and security.
related-rules:
  - architecture.md
  - security.md
allowed-tools: Read, Write, Edit, Bash
---

# API Design Skill

> **Expertise:** REST API design, HTTP semantics, versioning strategies, error contracts, OpenAPI, pagination, idempotency.

## HTTP Method & Status Code Reference

| Operation | Method | Success | Error cases |
|---|---|---|---|
| Create resource | POST | 201 Created | 400 Validation, 409 Conflict |
| Read resource | GET | 200 OK | 404 Not Found, 403 Forbidden |
| Full update | PUT | 200 OK | 400, 404, 409 |
| Partial update | PATCH | 200 OK | 400, 404 |
| Delete | DELETE | 204 No Content | 404, 409 (has dependents) |
| Async action | POST | 202 Accepted | 400 |

## URL Design Rules

```
✅ Nouns, plural, lowercase, kebab-case
   GET  /users/{id}
   POST /orders
   GET  /product-categories

❌ Verbs in path
   POST /createOrder
   GET  /getUser?id=123

✅ Nesting only for true ownership (max 2 levels)
   GET  /orders/{order_id}/items          ✅ items belong to order
   GET  /orders/{order_id}/items/{item_id}/tags   ❌ too deep

✅ Actions as sub-resources
   POST /orders/{id}/cancel
   POST /invoices/{id}/resend
```

## Standard Error Contract

All error responses must follow the same shape:
```json
{
  "error": {
    "code": "ORDER_NOT_FOUND",        // machine-readable, stable
    "message": "Order ord_123 not found",  // human-readable
    "details": [                       // optional: per-field errors
      { "field": "items[0].quantity", "issue": "must be > 0" }
    ],
    "request_id": "req_abc123"        // traceable
  }
}
```

```python
# FastAPI implementation
from fastapi import HTTPException
from pydantic import BaseModel
from typing import Optional, List

class ErrorDetail(BaseModel):
    field: str
    issue: str

class ErrorResponse(BaseModel):
    code: str
    message: str
    details: Optional[List[ErrorDetail]] = None
    request_id: Optional[str] = None

# Usage
raise HTTPException(
    status_code=404,
    detail=ErrorResponse(
        code="ORDER_NOT_FOUND",
        message=f"Order {order_id} not found",
        request_id=request.state.request_id
    ).model_dump()
)
```

## Pagination Patterns

### Cursor-based (preferred for large/live datasets)
```python
class PaginatedResponse(BaseModel, Generic[T]):
    items: List[T]
    next_cursor: Optional[str] = None   # opaque, base64-encoded
    total_count: Optional[int] = None   # only if cheap to compute

# Encode cursor: hide implementation detail from client
import base64, json

def encode_cursor(last_id: int, last_created_at: datetime) -> str:
    payload = {"id": last_id, "ts": last_created_at.isoformat()}
    return base64.b64encode(json.dumps(payload).encode()).decode()

def decode_cursor(cursor: str) -> dict:
    return json.loads(base64.b64decode(cursor).decode())
```

### Offset-based (only for small, static datasets)
```python
# Only when total count is cheap and dataset doesn't change under pagination
@router.get("/products")
async def list_products(page: int = 1, page_size: int = Query(20, le=100)):
    offset = (page - 1) * page_size
    # ...
```

## Versioning Strategy

```
URL versioning (recommended for major breaking changes):
  /api/v1/orders
  /api/v2/orders  ← new schema, old still supported

Header versioning (for minor variations):
  Accept: application/vnd.myapi.v2+json

Rules:
- v1 stays alive for minimum 6 months after v2 launch
- Deprecated endpoints return: Deprecation: true, Sunset: <date> headers
- Never remove a field from a response without a major version bump
```

## Idempotency

```python
# Idempotency key pattern for POST mutations
@router.post("/orders", status_code=201)
async def create_order(
    body: CreateOrderRequest,
    idempotency_key: Optional[str] = Header(None, alias="X-Idempotency-Key"),
    db: AsyncSession = Depends(get_db),
    redis: Redis = Depends(get_redis),
):
    if idempotency_key:
        cached = await redis.get(f"idempotency:{idempotency_key}")
        if cached:
            return JSONResponse(json.loads(cached), status_code=200)  # 200 = already done

    order = await order_service.create(db, body)
    response = OrderResponse.model_validate(order)

    if idempotency_key:
        await redis.setex(
            f"idempotency:{idempotency_key}",
            86400,  # 24h TTL
            response.model_dump_json()
        )
    return response
```

## Security Checklist

- [ ] Auth middleware applied before any handler logic — never inside handler
- [ ] Ownership check: `if order.user_id != current_user.id: raise 403`
- [ ] Input validation via Pydantic (FastAPI) or Joi/Zod (Node) on every endpoint
- [ ] Rate limiting on public endpoints (especially auth, password reset)
- [ ] No sensitive data in URLs (tokens, passwords) — use request body or headers
- [ ] CORS configured explicitly: no `allow_origins=["*"]` in production
