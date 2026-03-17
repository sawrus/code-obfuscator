# code-obfuscator

Практический гайд по использованию MCP-сервера для обфускации кода перед LLM и обратной деобфускации.

## Что делает MCP

Сервер экспортирует 2 инструмента:
- `obfuscate_project` — обфусцирует `project_files` перед отправкой в LLM.
- `deobfuscate_project` — восстанавливает ответ LLM по `mapping_payload` (или по default mapping сервера, если payload не передан).

## Шаг 1. Подготовить default mapping

Создайте файл `mapping.default.json`, например:

```json
{
  "business_secret": "bs"
}
```

Этот mapping используется только если в `obfuscate_project` не передан `manual_mapping`.

## Шаг 2. Собрать MCP

Локально:

```bash
cargo build --bin mcp-server
```

Docker-образ:

```bash
make mcp-docker-build
```

## Шаг 3. Выбрать транспорт в Codex

### Вариант A: HTTP MCP (`url`)

1. Запустите MCP с HTTP и mapping:

```bash
MCP_HTTP_ADDR=127.0.0.1:18787 \
MCP_DEFAULT_MAPPING_PATH=./mapping.default.json \
./scripts/run-mcp-docker.sh
```

2. Укажите сервер в `~/.codex/config.toml`:

```toml
[mcp_servers.code_obfuscator]
enabled = true
url = "http://127.0.0.1:18787"
```

### Вариант B: stdio MCP (`command/args`)

Укажите сервер в `~/.codex/config.toml`:

```toml
[mcp_servers.code_obfuscator]
enabled = true
command = "docker"
args = [
  "run", "--rm", "-i",
  "-e", "MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json",
  "-e", "MCP_LOG_STDOUT=false",
  "-v", "/ABS/PATH/mapping.default.json:/data/mapping.default.json",
  "code-obfuscator-mcp:local"
]
```

В stdio-режиме mapping читается из пути `MCP_DEFAULT_MAPPING_PATH` внутри контейнера (`/data/mapping.default.json`), который смонтирован с хоста.
Сервер поддерживает оба формата stdio framing: `Content-Length` и JSON-lines.

## Шаг 4. Проверить, что MCP доступен

Проверка в Codex:

```bash
codex mcp list
codex mcp get code_obfuscator
```

Проверка HTTP (если используете `url`):

```bash
curl -i http://127.0.0.1:18787/health
curl -i http://127.0.0.1:18787/mapping
```

## Шаг 5. Рекомендуемый workflow с LLM (обязательный pre-step)

1. Клиент читает исходные файлы проекта.
2. Клиент вызывает `obfuscate_project`.
3. В LLM отправляются только `obfuscated_files`.
4. После ответа LLM вызывается `deobfuscate_project`.

Важно: MCP не перехватывает автоматически любое локальное чтение файлов Codex. Pre-step должен быть явным в оркестрации.

## Контракт инструментов (кратко)

### `obfuscate_project`

Вход:
- `project_files: [{ path, content }]`
- `manual_mapping` (опционально)
- `options` (опционально):
  - `request_id`
  - `stream`
  - `enrich_detected_terms`

Поведение:
- По умолчанию `enrich_detected_terms = false`.
- В default-режиме используются только `manual_mapping` или server default mapping.
- Старое auto-enrich поведение включается только через `options.enrich_detected_terms = true`.

### `deobfuscate_project`

Вход:
- `llm_output_files: [{ path, content }]`
- `mapping_payload` (опционально)

Приоритет mapping:
1. `mapping_payload` из запроса.
2. fallback на server default mapping.

## HTTP endpoints

- `GET /health` — healthcheck.
- `GET /mapping` — текущий default mapping.
- `PUT /mapping` — обновить default mapping (`{"a":"b"}` или `{"mapping":{"a":"b"}}`).
- `POST /` — MCP JSON-RPC.
- `POST /mcp` — MCP JSON-RPC (alias).

## Логирование MCP

Логи пишутся в JSONL:
- в stdout,
- в `logs/mcp-server.log` с ротацией по умолчанию `10MB x 10`.

Переменные:
- `MCP_LOG_DIR` (default: `logs`)
- `MCP_LOG_MAX_BYTES` (default: `10485760`)
- `MCP_LOG_MAX_FILES` (default: `10`)
- `MCP_LOG_STDOUT` (default: `true`)

Пример запуска с явными лог-настройками:

```bash
MCP_HTTP_ADDR=127.0.0.1:18787 \
MCP_DEFAULT_MAPPING_PATH=~/projects/ai/codex/code-obfuscator/mapping.default.json \
MCP_LOG_DIR=~/projects/ai/codex/code-obfuscator/logs \
MCP_LOG_MAX_BYTES=10485760 \
MCP_LOG_MAX_FILES=10 \
cargo run --bin mcp-server
```

## Частые проблемы

1. `url` указан, но Codex не работает с MCP.
- Проверьте, что сервер реально слушает `MCP_HTTP_ADDR`.
- Проверьте `GET /health`.
- Убедитесь, что используете MCP endpoint `POST /` или `POST /mcp`.

2. `stdio + docker` даёт `timed out handshaking after 10s`.
- Частая причина: Codex-процесс не имеет доступа к Docker socket.
- Проверка: `docker run --rm -i ... code-obfuscator-mcp:local` в том же окружении.
- Если доступ к Docker ограничен, используйте `url = "http://127.0.0.1:18787"` и запускайте MCP отдельно (например, `./scripts/run-mcp-docker.sh`).

3. В результате нет замен из `mapping.default.json`.
- Проверьте, что `MCP_DEFAULT_MAPPING_PATH` указывает на существующий файл.
- В stdio+docker проверьте корректный `-v` mount.

4. Результат “слишком сильно” обфусцирован.
- Убедитесь, что `options.enrich_detected_terms` не включен.

## Полезные команды разработки

```bash
make build
make test
cargo test --test mcp_server
```
