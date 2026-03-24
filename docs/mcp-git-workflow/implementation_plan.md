# Implementation Plan — MCP Git-Style Refactor

## API Contract

### `ls_tree`
Input: `root_dir`, `max_depth?`, `max_entries?`, `include_hidden?`.
Output: `entries[{path, kind}]`, `truncated`.

### `ls_files`
Input: `root_dir`, `max_entries?`, `include_hidden?`.
Output: `files[]`, `truncated`.

### `pull`
Input: `root_dir`, `file_paths?`, `options.request_id` (+ `stream?`, `enrich_detected_terms?`).
Output: `obfuscated_files`, `stats`, `events`.
Behavior: reads source files in MCP, obfuscates, persists snapshot for `request_id`.

### `clone`
Input: `root_dir`, `workspace_dir`, `options.request_id`.
Output: `cloned_files`, `stats`, `events`.
Behavior: full-tree obfuscation + materialized workspace snapshot.

### `status`
Input: `workspace_dir`, `options.request_id`.
Output: `clean`, `diff{added,modified,deleted}`, `mapping_state`.
Behavior: compares workspace against request snapshot.

### `push`
Input: `workspace_dir`, `options.request_id`.
Output: `applied_files`, `deleted_files`, `stats`, `events`.
Behavior: deobfuscates changed files and applies add/modify/delete into source root.

## State Model
`request_id` stores:
1. Mapping payload.
2. Bound `root_dir`.
3. Optional bound `workspace_dir`.
4. Baseline obfuscated file hashes (for status/push diff).
5. Timestamps and TTL.

## Security Constraints
1. `request_id` required and validated.
2. Path traversal, absolute path, null-byte are rejected.
3. Symlink write/delete is rejected.
4. `clone` rejects non-empty workspace and `workspace_dir == root_dir`.
5. `push` applies only within bound source root.

## Performance Constraints
1. Tree listing limits remain bounded by existing max values.
2. Diff is hash-based and path-keyed.
3. Snapshot is refreshed after successful `push`.

## Migration Notes
1. Old tool names are removed (breaking change).
2. Env toggle `MCP_ALLOW_DIRECT_DEOBFUSCATION` is removed.
3. Docs, tests, and prompts must be migrated in the same increment.

## Design Constraints
1. Tool descriptions must distinguish source root vs workspace clearly.
2. Error messages must be actionable (what failed and next step).
3. Lifecycle: `clone|pull -> status -> push` with shared `request_id`.

## External Best-Practice References
1. MCP specification (Tools): [modelcontextprotocol.io/specification/2025-06-18/server/tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools).
2. Microsoft guidance for MCP tools and third-party MCP security considerations: [learn.microsoft.com/.../local-mcp-tools](https://learn.microsoft.com/en-us/agent-framework/agents/tools/local-mcp-tools).
3. Claude MCP connector docs (toolset/server naming and allowlist patterns): [platform.claude.com/docs/en/agents-and-tools/mcp-connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector).

### Applied to this refactor
1. Tool names follow MCP-safe naming and are unique in server scope: `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`.
2. Tool contracts are explicit JSON schemas with required fields and strict validation.
3. Operational safety follows human-in-the-loop/security expectations: path guards, request-scoped sessions, and explicit status before push.
4. Workflow is compatible with toolset allowlist/denylist operation from Claude/Microsoft agent runtimes.
