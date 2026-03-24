# Cross-IDE Playbooks (Codex, Claude, Gemini)

## Shared Prerequisites

1. MCP server exposes git-style tools only: `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`.
2. Source root is mounted read-write for `push` operations.
3. You can map a writable host mirror path for workspace edits.
4. One stable `request_id` is reused across all calls.

## Codex Playbook

### MCP server registration (example)

```bash
codex mcp remove code_obfuscator || true
codex mcp add code_obfuscator -- \
  docker run --rm -i \
  -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
  -e MCP_LOG_STDOUT=false \
  -v /abs/path/mapping.default.json:/data/mapping.default.json:ro \
  -v /abs/path/projects:/workspace/projects:rw \
  code-obfuscator-mcp:local
```

### Prompt scaffold

```text
Work only through MCP `code_obfuscator`.
Use one request_id=<REQ>.
Call in order: ls_tree -> clone -> status -> push -> status.
MCP paths:
- root_dir=/workspace/projects/test
- workspace_dir=/workspace/projects/test-obf
Edit obfuscated files through host mirror path:
- /abs/path/projects/test-obf
Before push require clean=false; after push require clean=true.
If any tool call fails, stop and return exact error.
```

### Codex-specific notes

1. In sandboxed mode, shell may not write `/workspace/...` directly.
2. Edit files via host mirror path mapped to `/workspace/projects`.
3. Keep debug checks concise to avoid unnecessary prompt drift.

## Claude Playbook

### Connector setup

1. Add MCP server in Claude MCP connector configuration.
2. Prefer explicit allowlist of tool names:
   - `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`
3. Provide runtime mount mapping documentation near the server config.

### Prompt scaffold

```text
Use MCP server `code_obfuscator` only.
Use one request_id=<REQ>.
Sequence is mandatory: ls_tree, clone, status, push, status.
Use MCP runtime paths for tool calls and host mirror path for direct file edits.
Stop on any failed tool call and print exact MCP error.
Final status must be clean=true.
```

### Claude-specific notes

1. Tool allowlist reduces accidental calls to legacy methods.
2. Keep instructions explicit about runtime path versus host mirror path.
3. Require a final structured summary: applied_files, deleted_files, final clean state.

## Gemini Playbook

### Connector/bridge setup

1. Use an MCP-compatible bridge/connector supported by your Gemini environment.
2. Register the same server command and mount model used in Codex/Claude.
3. Confirm tool schema compatibility for nested `options.request_id` payloads.

### Prompt scaffold

```text
Operate only with MCP tools:
ls_tree, ls_files, pull, clone, status, push.
Use request_id=<REQ> for all calls.
Clone workspace, edit obfuscated files only, validate status before and after push.
Use runtime MCP paths for tools and mapped host mirror path for file edits.
Abort on any MCP error and return exact message.
```

### Gemini-specific notes

1. Some bridges may sanitize arguments; validate `options.request_id` reaches server unchanged.
2. Keep arguments strict JSON without extra unknown fields.
3. If push fails, inspect mount permissions first.

## Universal Failure Recovery

1. `unknown request_id` -> recreate snapshot with new `clone` and reuse new id consistently.
2. `root_dir does not exist` -> fix mount/runtime path mismatch.
3. `root_dir is not writable for push` -> remount source as read-write.
4. `refusing to write through symlink` -> replace symlink path with regular file.
5. final `clean=false` after push -> inspect workspace edits and rerun `status` + `push`.

## Compliance Checklist For Any IDE

1. Used only git-style tools.
2. Reused one request id end-to-end.
3. Performed pre-push and post-push status checks.
4. Confirmed post-push `clean=true`.
5. Returned precise error details when failure occurred.
