# Test Report — MCP Git-Style Workflow

## Metadata
- Feature: `mcp-git-workflow`
- Scope: `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`
- Date: 2026-03-24
- QA Owner: `@qa`
- Branch/Commit: `main` (working tree, uncommitted)

## Scenario Catalog

### Happy Path
- HAP-01: `tools/list` exposes only new tools. Result: PASS (`tests/mcp_server.rs`).
- HAP-02: `pull` obfuscates selected subset. Result: PASS (`tests/mcp_server.rs`).
- HAP-03: `clone` writes full obfuscated workspace. Result: PASS (`tests/mcp_server.rs`).
- HAP-04: `status` reports added/modified/deleted. Result: PASS (`tests/mcp_server.rs` + unit tests in `src/bin/mcp-server.rs`).
- HAP-05: `push` applies add/modify/delete correctly. Result: PASS (`tests/mcp_server.rs`).
- HAP-06: HTTP JSON-RPC path supports new tools. Result: PASS (`tests/mcp_server.rs`).

### Edge
- EDG-01: Empty tree handling. Result: PASS (covered by listing logic + blackbox flow).
- EDG-02: Hidden files include/exclude behavior. Result: PASS (`tests/mcp_server.rs::mcp_ls_tree_and_ls_files_respect_hidden_and_truncation`).
- EDG-03: Truncation behavior for tree/file listing. Result: PASS (`tests/mcp_server.rs::mcp_ls_tree_and_ls_files_respect_hidden_and_truncation`).
- EDG-04: Clone rejects invalid workspace target. Result: PASS (unit coverage `prepare_workspace_dir_rejects_root_dir`).

### Failure
- FLR-01: Legacy tool names rejected. Result: PASS (`tests/mcp_server.rs`).
- FLR-02: Unknown request snapshot rejected by push/status. Result: PASS (`tests/security_tests.rs`).
- FLR-03: Invalid args/unknown fields rejected. Result: PASS (JSON-RPC argument validation behavior retained; integration checks green).

### Security
- SEC-01: Null-byte/path traversal rejected. Result: PASS (`tests/security_tests.rs`).
- SEC-02: Oversized HTTP body rejected. Result: PASS (`tests/security_tests.rs`).
- SEC-03: Missing signature/encryption keys rejected when options enabled. Result: PASS (`tests/security_tests.rs`).
- SEC-04: Push rejects write-through source symlink. Result: PASS (`tests/security_tests.rs::push_rejects_writing_through_source_symlink`).

## Command Matrix
- CMD-UNIT: `cargo test --bins` -> PASS
- CMD-INT: `cargo test --test mcp_server -- --nocapture` -> PASS
- CMD-SEC: `cargo test --test security_tests -- --nocapture` -> PASS
- CMD-BB: `bash scripts/e2e_blackbox.sh` -> PASS
- CMD-GATE: `make lint && make test && make e2e-blackbox` -> PASS

## Severity Rubric
- Critical: data loss/security bypass/out-of-root write-delete.
- High: broken core `pull/clone/status/push` behavior.
- Medium: non-core functional mismatch.
- Low: docs/cosmetic issues.

## Defect Summary
- Open Critical: 0
- Open High: 0
- Open Medium: 0
- Open Low: 0

## GO/NO-GO Rule
NO-GO if any open Critical or High defect remains, or CMD-GATE fails.

## QA Recommendation
GO. No open Critical/High defects; all required gates are green.
