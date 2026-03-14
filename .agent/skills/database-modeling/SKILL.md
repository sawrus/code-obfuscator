---
name: database-modeling
type: skill
description: Design relational schemas, write efficient queries, plan indexes, and implement safe migrations.
related-rules:
  - data_access.md
  - architecture.md
allowed-tools: Read, Write, Edit, Bash
---

# Database Modeling Skill

> **Expertise:** PostgreSQL schema design, SQLAlchemy (async), query optimization, indexing, migrations (Alembic), safe schema changes.

## Schema Design Patterns

### Standard column set (all tables)
```python
from sqlalchemy import Column, Integer, DateTime, func
from sqlalchemy.orm import DeclarativeBase

class Base(DeclarativeBase):
    pass

class TimestampMixin:
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), server_default=func.now(), nullable=False
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), server_default=func.now(),
        onupdate=func.now(), nullable=False
    )

class Order(TimestampMixin, Base):
    __tablename__ = "orders"

    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id"), nullable=False, index=True)
    status: Mapped[str] = mapped_column(String(20), nullable=False, default="pending")
    total_amount: Mapped[Decimal] = mapped_column(Numeric(12, 2), nullable=False)
```

### Soft delete pattern
```python
class SoftDeleteMixin:
    deleted_at: Mapped[Optional[datetime]] = mapped_column(DateTime(timezone=True), nullable=True)

    @property
    def is_deleted(self) -> bool:
        return self.deleted_at is not None

# Always filter in repository, never expose deleted records by default
class OrderRepository:
    async def list_active(self, session: AsyncSession):
        return await session.execute(
            select(Order).where(Order.deleted_at.is_(None))
        )
```

## Indexing Strategy

```sql
-- Single column: high-cardinality columns used in WHERE/JOIN/ORDER BY
CREATE INDEX idx_orders_user_id ON orders(user_id);
CREATE INDEX idx_orders_status ON orders(status) WHERE deleted_at IS NULL;  -- partial index

-- Composite: query uses both columns together (order matters: equality first, then range)
CREATE INDEX idx_orders_user_created ON orders(user_id, created_at DESC);

-- Full-text search
CREATE INDEX idx_products_search ON products USING gin(to_tsvector('english', name || ' ' || description));

-- Never index: low-cardinality boolean columns, small tables (<1000 rows)
```

### EXPLAIN ANALYZE checklist
```sql
-- Run before every new query on a table with >10k rows
EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
SELECT * FROM orders WHERE user_id = 123 ORDER BY created_at DESC LIMIT 20;

-- Watch for: Seq Scan on large table → add index
--            Index Scan with high actual rows >> estimated rows → ANALYZE the table
--            Nested Loop with large inner side → consider Hash Join
```

## Repository Pattern

```python
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy import select, update

class OrderRepository:
    def __init__(self, session: AsyncSession):
        self.session = session

    async def get_by_id(self, order_id: int) -> Optional[Order]:
        result = await self.session.execute(
            select(Order).where(Order.id == order_id, Order.deleted_at.is_(None))
        )
        return result.scalar_one_or_none()

    async def list_by_user(
        self, user_id: int, *, limit: int = 20, cursor_id: Optional[int] = None
    ) -> list[Order]:
        q = select(Order).where(Order.user_id == user_id, Order.deleted_at.is_(None))
        if cursor_id:
            q = q.where(Order.id < cursor_id)  # cursor-based pagination
        q = q.order_by(Order.id.desc()).limit(limit)
        result = await self.session.execute(q)
        return list(result.scalars())

    async def update_status(self, order_id: int, status: str) -> None:
        await self.session.execute(
            update(Order).where(Order.id == order_id).values(status=status)
        )
        # No commit here — caller (service layer) owns the transaction
```

## Migration Safety (Alembic)

```bash
# Generate migration
alembic revision --autogenerate -m "add_index_orders_user_id"

# ALWAYS review generated file before applying
alembic show head

# Apply
alembic upgrade head

# Rollback one step
alembic downgrade -1
```

### Safe vs. unsafe schema operations

| Operation | Safe to deploy | Strategy |
|---|---|---|
| Add nullable column | ✅ Non-breaking | Apply directly |
| Add column with default | ✅ (PostgreSQL 11+) | Apply directly |
| Add NOT NULL column | ⚠️ Breaking | Add nullable → backfill → add constraint |
| Add index | ✅ with CONCURRENTLY | `CREATE INDEX CONCURRENTLY` |
| Rename column | ❌ Breaking | Expand/contract (add new → migrate code → drop old) |
| Drop column | ❌ Breaking | Deprecate in code → drop in next release |
| Change type | ❌ Breaking | Add new column with new type → migrate → drop old |

```python
# Alembic: create index without locking table
def upgrade():
    op.execute("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_orders_user_id ON orders(user_id)")

def downgrade():
    op.execute("DROP INDEX CONCURRENTLY IF EXISTS idx_orders_user_id")
```

## N+1 Query Prevention

```python
# ❌ N+1: loads orders, then 1 query per order to get user
orders = await session.execute(select(Order)).scalars()
for order in orders:
    print(order.user.name)  # each access fires a query

# ✅ Eager load with joinedload
from sqlalchemy.orm import joinedload

orders = await session.execute(
    select(Order)
    .options(joinedload(Order.user))  # single JOIN
    .where(Order.status == "pending")
)
```
