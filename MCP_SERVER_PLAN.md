# MCP server plan for code obfuscation pipeline (without `--deep`)

## Goal
Build an MCP server with optional server-side default mapping that:
1. receives a project tree (text files only),
2. obfuscates it before handing to LLM,
3. receives LLM output,
4. deobfuscates output back for the client.

Current stage explicitly excludes deep mode (`--deep`).

## Confirmed requirements
- MCP may work with full project trees (including projects cloned from Git).
- Binary/non-text files are out of scope.
- Hybrid model: client may send mapping back for deobfuscation, but server may also use default mapping fallback.
- Returning mapping in open/plain form is allowed.
- Optional security extensions are desired: signature/TTL/encryption as opt-in.
- Limits target: up to 1000 projects, each up to 1,000,000 files.
- On inconsistent LLM output, deobfuscation should fail fast.
- `--deep` is not required in this phase.
- Streaming support is required.
- Audit storage: metadata only (no raw code persistence).
- Orchestration model: MCP mediates between client and LLM (obfuscate before LLM, deobfuscate after LLM).

## Proposed MCP interface

### Tool 1: `obfuscate_project`
Input:
- `project_files`: list of `{ path, content }` (UTF-8 text only)
- `manual_mapping` (optional)
- `options` (optional):
  - `request_id`
  - `security`: `{ sign_mapping?: bool, ttl_seconds?: u32, encrypt_mapping?: bool }`

Output:
- `obfuscated_files`: list of `{ path, content }`
- `mapping_payload`: opaque payload for reverse step (plain JSON by default)
- `stats`: `{ file_count, mapping_entries, elapsed_ms }`

Behavior:
- Writes input files into temp source dir.
- Runs `code-obfuscator --mode forward --source ... --target ...` without `--deep`.
- Returns transformed files and generated mapping.

### Tool 2: `deobfuscate_project`
Input:
- `llm_output_files`: list of `{ path, content }`
- `mapping_payload` (optional; if missing, server may use default mapping)
- `options` (optional): `{ request_id }`

Output:
- `restored_files`: list of `{ path, content }`
- `stats`: `{ file_count, restored_tokens, elapsed_ms }`

Behavior:
- Validates payload integrity/format.
- Writes files + mapping to temp dirs.
- Runs `code-obfuscator --mode reverse --source ... --target ... --mapping ...` without `--deep`.
- Fail-fast if mapping is invalid or required obfuscated tokens are missing.

### Streaming layer
Add optional stream events for long runs:
- `queued`
- `scanning`
- `obfuscating` / `deobfuscating`
- `collecting_results`
- `completed`
- `failed`

Each event should include `request_id`, `stage`, `progress` (if measurable), and timestamps.

## Architecture
1. **MCP transport adapter**: handles tool schema + streaming.
2. **Validation module**:
   - UTF-8 text enforcement,
   - path normalization and traversal prevention,
   - limits check (`projects`, `files`).
3. **Workspace manager**:
   - per-request temporary directories,
   - deterministic cleanup.
4. **Obfuscator runner**:
   - invokes Rust CLI as subprocess,
   - enforces timeout,
   - captures structured stderr/stdout.
5. **Mapping payload module**:
   - plain payload default,
   - optional sign/encrypt/ttl wrappers.
6. **Metadata logger**:
   - request_id, counts, durations, exit status,
   - no source-code payload logging.

## Limits and reliability strategy
- Hard reject non-text files.
- Guardrails:
  - max projects per batch: `<= 1000`,
  - max files per project: `<= 1_000_000`.
- Chunked processing internally to avoid memory spikes on huge trees.
- Subprocess timeout + cancellation handling.
- Fail-fast reverse path on mapping mismatch.

## Security profile
Baseline:
- Hybrid model with optional server-side default mapping.
- Mapping returned to client in open form.

Optional controls (feature flags):
- HMAC signing for mapping payload integrity.
- TTL embedded in mapping envelope.
- Envelope encryption for mapping payload.

## Delivery roadmap
1. **Phase 1 (MVP)**
   - Two MCP tools (obfuscate/deobfuscate), non-streaming fallback,
   - text-only tree support,
   - default server mapping + plain mapping payload support,
   - metadata logs.
2. **Phase 2**
   - streaming progress events,
   - robust fail-fast diagnostics,
   - large-tree chunk optimization.
3. **Phase 3**
   - optional signing/TTL/encryption,
   - load tests for upper-bound limits,
   - operational dashboards on metadata.

## Acceptance criteria for MVP
- Round-trip correctness for representative text-only multi-file projects.
- Deobfuscation fails with explicit error on invalid or mismatched mapping.
- No raw code is persisted in logs.
- Works without using `--deep`.
