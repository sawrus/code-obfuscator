# code-obfuscator

CLI-утилита для обфускации исходного кода перед отправкой в AI-агента и обратной деобфускации после обработки.

## Возможности

- Режим `forward`: обфускация из исходной директории в целевую.
- Режим `reverse`: восстановление исходных терминов из обфусцированного кода.
- Опциональный `mapping.json` с ручными правилами (`Freeze -> Go`, `Antifraud -> Apple`).
- Опциональная интеграция с Ollama API для предложения дополнительных замен.
- Генерация `mapping.generated.json` для обратного преобразования.
- Кроссплатформенная сборка бинарника (macOS/Linux/Windows).

## Быстрый старт

```bash
cargo build
```

### Обфускация

```bash
./target/debug/code-obfuscator \
  --mode forward \
  --source ./project-src \
  --target ./project-obf \
  --mapping ./mapping.json
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

Если заданы `--ollama-url` и `--ollama-model`, утилита отправляет часть обнаруженных терминов в Ollama и добавляет ответы в карту замен.

## Makefile команды

- `make build` - сборка.
- `make test` - unit + e2e.
- `make e2e` - только e2e.
- `make svt` - нагрузочный blackbox тест (`ignored` по умолчанию).
- `make coverage` - отчёт покрытия через `cargo llvm-cov`.
- `make release-cross` - сборка для macOS/Linux/Windows targets.

Перед первым `make coverage` установите инструмент:

```bash
cargo install cargo-llvm-cov
```

## Тесты

- Unit тесты: в модулях (`src/*.rs`).
- E2E тесты: `tests/e2e.rs`.
- SVT тесты: `tests/svt.rs` (`#[ignore]`, запускаются вручную).

Целевое покрытие: >= 70% (контролируется в CI через `make coverage`).

## План интеграции

Утилита подходит для запуска из:

- AI Agent Skill
- MCP Server

Достаточно вызывать бинарник с нужными аргументами режимов.
