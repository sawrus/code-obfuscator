# CLI and Mapping Reference

Use this file when command flags or mapping behavior are needed.

## Binary location

Prefer `/Users/isaev/projects/ai/codex/upstream-code-obfuscator/target/debug/code-obfuscator`.
If the binary does not exist, run `cargo build` in `/Users/isaev/projects/ai/codex/upstream-code-obfuscator`.

## Core commands

Forward obfuscation:

```bash
code-obfuscator \
  --mode forward \
  --source <source_dir> \
  --target <target_dir> \
  --mapping <optional_mapping_json>
```

Reverse restoration:

```bash
code-obfuscator \
  --mode reverse \
  --source <obfuscated_dir> \
  --target <restored_dir> \
  --mapping <mapping_generated_json>
```

## Mapping rules

- Treat `mapping.json` as optional manual overrides.
- Treat `mapping.generated.json` as required input for reverse mode.
- Do not overwrite user-owned mapping files unless requested.
- Write generated mapping to `--output_mapping` when deterministic output path is required.

## Optional Ollama integration

Use only when the user explicitly requests AI-proposed replacements:

```bash
code-obfuscator \
  --mode forward \
  --source <source_dir> \
  --target <target_dir> \
  --ollama-url http://localhost:11434 \
  --ollama-model llama3.1 \
  --ollama-top-n 40
```

## Verification expectations

- After forward run, confirm target directory exists and contains transformed files.
- After reverse run, compare restored files with original source when round-trip validation is requested.
- If build/test commands are available, run at least one fast check after obfuscation.
