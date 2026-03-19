# General MCP Prompt Templates (RU / EN)

Ниже 4 готовых шаблона: короткий и длинный на русском и английском.

## RU — Short

```text
Работай только через MCP `code_obfuscator`.
Сделай задачу end-to-end в `root_dir="<ROOT_DIR>"` с `options.request_id="<REQUEST_ID>"`: `tools/list` -> `list_project_tree` -> `obfuscate_project_from_paths` -> правки только в `obfuscated_files` -> `apply_llm_output` (только измененный subset) -> проверки (test/lint/build) до зелёного.
Нельзя: прямое чтение/запись файлов проекта, `deobfuscate_project*`, клиентские mapping-аргументы.
При любой ошибке MCP/tool-call остановись и выведи точную ошибку.
В финале: `applied_files`, что изменено по смыслу, какие проверки запущены и их итог.
```

## RU — Long

```text
Работай только через MCP `code_obfuscator`.

Цель: выполнить разработку в проекте `<PROJECT_NAME>` через MCP по циклу анализ -> изменения -> применение -> проверка.

Обязательные правила:
1) Не читай и не изменяй файлы проекта напрямую.
2) Используй только MCP-инструменты:
   - `list_project_tree`
   - `obfuscate_project_from_paths`
   - `apply_llm_output`
3) Не используй `deobfuscate_project*`.
4) Не передавай клиентские mapping-аргументы.
5) Используй `root_dir="<ROOT_DIR>"`.
6) Используй один и тот же `options.request_id="<REQUEST_ID>"` для всей задачи.
7) В `apply_llm_output` передавай только изменённые файлы (subset из `obfuscated_files`).
8) После каждого применения запускай релевантные проверки (test/lint/build); при падении исправляй и повторяй цикл.
9) Если MCP недоступен, нет прав записи или tool-call падает — остановись и выведи точную ошибку.

Порядок:
1) `tools/list`
2) `list_project_tree(root_dir=...)`
3) `obfuscate_project_from_paths(root_dir=..., file_paths=[...], options.request_id=...)`
4) Измени только `obfuscated_files`
5) `apply_llm_output(root_dir=..., llm_output_files=[...], options.request_id=...)`
6) Повтори шаги 3-5 при необходимости до зелёных проверок

В финальном ответе покажи:
- `applied_files`
- кратко, что изменено по смыслу
- какие проверки были запущены и их итог.
```

## EN — Short

```text
Work only through MCP `code_obfuscator`.
Complete the task end-to-end in `root_dir="<ROOT_DIR>"` with `options.request_id="<REQUEST_ID>"`: `tools/list` -> `list_project_tree` -> `obfuscate_project_from_paths` -> edit only `obfuscated_files` -> `apply_llm_output` (changed subset only) -> run checks (test/lint/build) until green.
Forbidden: direct project file reads/writes, `deobfuscate_project*`, client-side mapping arguments.
If any MCP/tool call fails, stop and return the exact error.
Final output must include: `applied_files`, semantic change summary, executed checks and results.
```

## EN — Long

```text
Work only through MCP `code_obfuscator`.

Goal: implement the requested project change in `<PROJECT_NAME>` via MCP end-to-end: analyze -> edit -> apply -> verify.

Mandatory rules:
1) Do not read or write project files directly.
2) Use only these MCP tools:
   - `list_project_tree`
   - `obfuscate_project_from_paths`
   - `apply_llm_output`
3) Do not use `deobfuscate_project*`.
4) Do not send client-side mapping arguments.
5) Use `root_dir="<ROOT_DIR>"`.
6) Reuse the same `options.request_id="<REQUEST_ID>"` for the whole task.
7) Send only changed files to `apply_llm_output` (subset of `obfuscated_files`).
8) After each apply step, run relevant checks (test/lint/build); if failing, fix and iterate.
9) If MCP is unavailable, write access is denied, or any tool-call fails, stop and return the exact error.

Execution order:
1) `tools/list`
2) `list_project_tree(root_dir=...)`
3) `obfuscate_project_from_paths(root_dir=..., file_paths=[...], options.request_id=...)`
4) Edit only `obfuscated_files`
5) `apply_llm_output(root_dir=..., llm_output_files=[...], options.request_id=...)`
6) Repeat steps 3-5 until checks are green

Final response must include:
- `applied_files`
- a concise semantic summary of changes
- checks executed and their outcomes.
```
