# code-obfuscator

CLI-утилита для обфускации исходного кода перед отправкой в AI-агента и обратной деобфускации после обработки.

## Возможности

- Режим `forward`: обфускация из исходной директории в целевую.
- Режим `reverse`: восстановление исходных терминов из обфусцированного кода.
- Language-aware распознавание 10 языков: Python, JavaScript, TypeScript, Java, C#, C/C++, Go, Rust, SQL, Bash.
- Поддержка PostgreSQL SQL-лексики с совместимостью с ANSI/MySQL ключевыми словами.
- Глубокая SQL-обфускация идентификаторов таблиц и полей.
- Глубокая Python-обфускация переменных/констант/методов по карте замен.
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

