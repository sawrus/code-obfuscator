# Samples

## 1) Python: obfuscate + reverse

### Source
```python
def calculate_risk(customer_id):
    # customer_id is sensitive
    print(f"risk for {customer_id}")
```

### Obfuscated (example)
```python
def Cedar1032(Atlas7721):
    # Atlas7721 is sensitive
    print(f"risk for {Atlas7721}")
```

### Restored
```python
def calculate_risk(customer_id):
    # customer_id is sensitive
    print(f"risk for {customer_id}")
```

## 2) SQL (PostgreSQL compatible)

### Source
```sql
CREATE TABLE customer_orders (
  customer_id INT,
  order_total NUMERIC
);

SELECT customer_id FROM customer_orders;
```

### Obfuscated (example)
```sql
CREATE TABLE River4488 (
  Falcon9934 INT,
  Maple1882 NUMERIC
);

SELECT Falcon9934 FROM River4488;
```

### Restored
The reverse mode uses `mapping.generated.json` and restores original names one-to-one.

## 3) Bash + comments + strings

### Source
```bash
#!/usr/bin/env bash
business_handler() {
  local customer_name="$1"
  echo "customer_name=${customer_name}"
}
```

### Obfuscated (example)
```bash
#!/usr/bin/env bash
Comet3201() {
  local Quartz7733="$1"
  echo "Quartz7733=${Quartz7733}"
}
```

## 4) Ollama best-effort flow

```bash
./target/debug/code-obfuscator \
  --mode forward \
  --source ./project-src \
  --target ./project-obf \
  --ollama-url http://localhost:11434 \
  --ollama-model llama3.1 \
  --ollama-top-n 40
```

If Ollama returns valid JSON mapping (`old -> new`), it is merged into final mapping; otherwise random fallback mapping keeps pipeline stable.
