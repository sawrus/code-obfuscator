---
name: code-obfuscator
description: Obfuscate source code trees and reliably restore them using mapping files. Use when tasks involve running or validating the `code-obfuscator` CLI in forward/reverse modes, preparing mapping files, executing round-trip checks, or troubleshooting obfuscation output for this repository.
---

# Code Obfuscator

## Overview

Use this skill to run deterministic obfuscation workflows against the local project at `/Users/isaev/projects/ai/codex/upstream-code-obfuscator`.

## Workflow

1. Confirm inputs.
- Validate `--source` exists.
- Select `--target` path that is safe to create or overwrite.
- Use manual `mapping.json` only when provided by the user.

2. Ensure binary is available.
- Prefer `/Users/isaev/projects/ai/codex/upstream-code-obfuscator/target/debug/code-obfuscator`.
- Build with `cargo build` in `/Users/isaev/projects/ai/codex/upstream-code-obfuscator` when missing.

3. Run forward or reverse mode.
- Use `--mode forward` for obfuscation.
- Use `--mode reverse` with generated mapping for restoration.
- If the user asks for deterministic generated-mapping path, pass `--output_mapping`.

4. Verify outcomes.
- Confirm expected files appear in target.
- For round-trip tasks, run forward + reverse and compare restored tree with source.
- Run a fast project check (`cargo test`, specific test target, or user-provided smoke test) when requested.

## Safety Rules

- Do not delete source directories.
- Do not overwrite user mapping files unless explicitly requested.
- Use Ollama flags only when user explicitly asks for AI-generated replacement candidates.
- Surface non-reversible changes immediately if round-trip comparison fails.

## Bundled Resources

- Use `references/cli-and-mapping.md` for full flag and mapping behavior details.
- Use `scripts/obfuscate_roundtrip.sh` when the user requests a quick end-to-end validation run.
