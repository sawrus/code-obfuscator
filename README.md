# code-obfuscator

CLI-утилита для обфускации исходного кода перед отправкой в AI-агента и обратной деобфускации после обработки.

## Возможности

- Режим `forward`: обфускация из исходной директории в целевую.
- Режим `reverse`: восстановление исходных терминов из обфусцированного кода.
- Language-aware распознавание 10 языков: Python, JavaScript, TypeScript, Java, C#, C/C++, Go, Rust, SQL, Bash.
- Поддержка PostgreSQL SQL-лексики с совместимостью с ANSI/MySQL ключевыми словами.
- Флаг `--deep` включает language-aware обфускацию (SQL/Python и др.).
- По умолчанию выполняется только глобальная whole-word замена по `--mapping` во всех файлах проекта.
- Защита Python-импортов и builtins: внешние классы/символы не переименовываются.
- Обфускация идентификаторов в коде, строках и комментариях (по whole-word замене).
- Опциональный `mapping.json` с ручными правилами (`Freeze -> Go`, `Antifraud -> Apple`).
- Опциональная интеграция с Ollama API для предложения дополнительных замен (best-effort).
- Генерация `mapping.generated.json` для обратного преобразования.
- Кроссплатформенная сборка бинарника (macOS/Linux/Windows).

## Быстрый старт

```bash
make build
```

### Обфускация

```bash
./target/debug/code-obfuscator \
  --mode forward \
  --source ./project-src \
  --target ./project-obf \
  --mapping ./mapping.json
```

> По умолчанию (без `--deep`) используется только глобальная замена по `mapping.json`.


### Глубокая language-aware обфускация

```bash
./target/debug/code-obfuscator \
  --mode forward \
  --source ./project-src \
  --target ./project-obf \
  --mapping ./mapping.json \
  --deep
```

### Деобфускация

```bash
./target/debug/code-obfuscator \
  --mode reverse \
  --source ./project-obf \
  --target ./project-restored \
  --mapping ./project-obf/mapping.generated.json
```

## Формат mapping.json

```json
{
  "Freeze": "Go",
  "Antifraud": "Apple"
}
```

## Ollama (опционально)

```bash
./target/debug/code-obfuscator \
  --mode forward \
  --source ./project-src \
  --target ./project-obf \
  --ollama-url http://localhost:11434 \
  --ollama-model llama3.1 \
  --ollama-top-n 40
```

Если заданы `--ollama-url` и `--ollama-model`, утилита отправляет часть language-aware обнаруженных терминов в Ollama и добавляет валидные ответы в карту замен.


## MCP server (stdio)

В проекте добавлен отдельный MCP-серверный бинарник `mcp-server`, который реализует flow:

1. `obfuscate_project` — обфускация дерева текстовых файлов перед отправкой в LLM (без `--deep`).
2. `deobfuscate_project` — обратное восстановление результата LLM по `mapping_payload` или по default mapping сервера (если payload не передан).

Запуск:

```bash
cargo run --bin mcp-server
```

Сервер работает по MCP/JSON-RPC через `stdio` (`Content-Length` framing) и экспортирует tools:
- `obfuscate_project`
- `deobfuscate_project`

### Запуск MCP через Docker

Сборка и запуск из Docker:

```bash
make mcp-docker-run
```

Или напрямую:

```bash
./scripts/run-mcp-docker.sh
```

Для включения HTTP API и дефолтного mapping в Docker-режиме:

```bash
MCP_HTTP_ADDR=127.0.0.1:18787 \
MCP_DEFAULT_MAPPING_PATH=./mapping.default.json \
./scripts/run-mcp-docker.sh
```

Также доступны отдельные шаги:

```bash
make mcp-docker-build
docker run --rm -i code-obfuscator-mcp:local
```

### Подключение локального MCP в Codex CLI

Добавьте сервер в конфиг Codex CLI (пример `~/.codex/config.toml`):

```toml
[mcp_servers.code_obfuscator]
command = "docker"
args = [
  "run", "--rm", "-i",
  "code-obfuscator-mcp:local"
]
```

После этого перезапустите Codex CLI и проверьте, что сервер доступен в списке MCP-серверов.

Если хотите запускать без Docker, можно указать бинарник напрямую:

```toml
[mcp_servers.code_obfuscator]
command = "cargo"
args = ["run", "--manifest-path", "/ABS/PATH/code-obfuscator/Cargo.toml", "--bin", "mcp-server"]
```

В такой конфигурации LLM (включая GPT-5.x в Codex CLI) будет вызывать инструменты `obfuscate_project`/`deobfuscate_project` через ваш локальный MCP-сервер.

### Default mapping на стороне MCP + HTTP API

Сервер поддерживает дефолтный mapping, который применяется в `obfuscate_project`, если `manual_mapping` не передан клиентом.

Переменные окружения:

- `MCP_DEFAULT_MAPPING_PATH` — путь к JSON-файлу дефолтного mapping (например `./mapping.default.json`).
- `MCP_HTTP_ADDR` — адрес HTTP API для runtime-управления mapping (например `127.0.0.1:18787`).

Пример запуска:

```bash
MCP_DEFAULT_MAPPING_PATH=./mapping.default.json \
MCP_HTTP_ADDR=127.0.0.1:18787 \
cargo run --bin mcp-server
```

HTTP endpoints:

- `GET /health` — healthcheck.
- `GET /mapping` — получить текущий default mapping.
- `PUT /mapping` — обновить default mapping.
  - body может быть либо JSON-object mapping (`{"a":"b"}`),
  - либо envelope (`{"mapping": {"a":"b"}}`).

После `PUT /mapping` состояние обновляется в памяти и сохраняется в `MCP_DEFAULT_MAPPING_PATH` (если путь задан).

Важно: приоритет для деобфускации такой: (1) `mapping_payload` из запроса, (2) fallback на default mapping сервера. Рекомендуемый и наиболее точный путь — передавать `mapping_payload` из `obfuscate_project`, чтобы восстановить именно тот вариант обфускации, который ушёл в LLM.

Ограничения текущей версии MCP:
- только текстовые файлы (`path + content`),
- гибридная модель: и `obfuscate_project`, и `deobfuscate_project` могут использовать server-side default mapping; если передан `mapping_payload`, он имеет приоритет,
- fail-fast при деобфускации, если обязательные обфусцированные токены отсутствуют в ответе LLM,
- `--deep` не используется.

## Makefile команды

- `make build` - сборка.
- `make test` - unit + e2e.
- `make e2e` - только e2e.
- `make svt` - нагрузочный blackbox тест (`ignored` по умолчанию, запуск вручную).
- `make coverage` - отчёт покрытия через `cargo llvm-cov`.
- `make release-cross` - сборка для macOS/Linux/Windows targets.
- `make release-artifacts` - упаковка бинарников в `dist/`.
- `make ci` - fmt + clippy + test + coverage.

Перед первым `make coverage` установите инструмент:

```bash
cargo install cargo-llvm-cov
```

## CI/CD

- `.github/workflows/ci.yml`: запускает Makefile-цель `make ci`.
- `.github/workflows/release.yml`: по git-тегам `v*` публикует бинарники для Linux/Windows/macOS.

## Тесты

- Unit тесты: в модулях (`src/*.rs`) для language-aware логики.
- E2E тесты: `tests/e2e.rs`, включая roundtrip для 10 языков и запуск доступных компиляторов/интерпретаторов.
- SVT тесты: `tests/svt.rs` (`#[ignore]`, manual run).

Целевое покрытие: >= 70% (контролируется в CI через `make coverage`).

## Бизнес-кейсы

- Безопасная передача кода в внешние LLM/AI-агенты без раскрытия доменных терминов.
- Поддержка monorepo с несколькими языками без раздельных тулов на каждый язык.
- Обратимая обфускация для безопасной интеграции AI-правок обратно в продуктовые репозитории.

См. `SAMPLES.md` для примеров до/после.



## Full compiler/runtime E2E environment

`tests/e2e.rs` runs real compile/runtime checks for Python, JavaScript/TypeScript, Java, C++, Go, Rust, SQL, Bash and C# when toolchains are present.

To run with a fully provisioned toolchain container (including .NET for C#), use:

```bash
./scripts/run-e2e-full.sh
```

This script builds `docker/e2e.Dockerfile` and executes `cargo test --tests` inside that image.
