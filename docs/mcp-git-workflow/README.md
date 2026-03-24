# MCP Git-Style Workflow

## Problem Statement
Current MCP methods were not aligned with the common git mental model. This increased usage errors in Agent IDE flows and made prompts/docs harder to operate consistently.

## Expected Outcomes
1. MCP API is exposed as `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push` only.
2. `clone -> edit -> status -> push` becomes the default predictable workflow.
3. Old MCP method names are removed and fail fast.
4. Unit, integration/e2e, and blackbox tests are updated for the new flow.

## Acceptance Criteria
1. `tools/list` returns only `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`.
2. `pull` obfuscates `root_dir + file_paths?`, returns obfuscated payload, and stores request snapshot.
3. `clone` obfuscates full source tree and writes into `workspace_dir`.
4. `status` reports `added`, `modified`, `deleted`, and mapping/session state.
5. `push` applies `add/modify/delete` from workspace back to source root with safety checks.
6. Legacy tools (`list_project_tree`, `obfuscate_*`, `apply_llm_output`, `deobfuscate_*`) are unavailable.
7. Required checks pass: `make lint`, `make test`, `make e2e-blackbox`.

## Non-Goals
1. Backward-compatible aliases for old MCP tool names.
2. New transports or APIs beyond current JSON-RPC MCP envelope.
3. Binary file support in obfuscation/apply flows.

## Scope Boundaries
1. Transport behavior (`stdio`/HTTP endpoints) remains unchanged.
2. Mapping storage remains request-scoped with TTL and max entries.
3. Security path guards remain mandatory for read/write/delete operations.

## Success Metrics
1. New tool set is the only public MCP surface.
2. Old method calls fail explicitly.
3. New workflow tests pass across required layers.
4. Docs and prompts reflect runtime behavior exactly.
