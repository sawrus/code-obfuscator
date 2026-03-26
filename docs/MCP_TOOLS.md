# MCP server plan for git-style workflow

## Goal
Expose MCP tools that mirror a git-like workflow for obfuscated editing:
1. inspect files (`ls_tree`, `ls_files`),
2. fetch obfuscated view (`pull` or full `clone`),
3. inspect workspace delta (`status`),
4. apply delta back (`push`).

## Tool surface
- `ls_tree(root_dir, max_depth?, max_entries?, include_hidden?)`
- `ls_files(root_dir, max_entries?, include_hidden?)`
- `pull(root_dir, file_paths?, options.request_id)`
- `clone(root_dir, workspace_dir, options.request_id)`
- `status(workspace_dir, options.request_id)`
- `push(workspace_dir, options.request_id)`

## State model
Each `request_id` stores mapping payload and session context:
- source root binding,
- optional workspace binding,
- baseline obfuscated file hashes for `status`/`push` diff,
- TTL/max-entry lifecycle.

## Safety
- path traversal/null-byte/absolute path checks,
- symlink write/delete refusal,
- clone target must be empty and different from source root,
- push applies only within bound source root.

## File selection rules
- MCP tools respect `<root_dir>/.gitignore` when scanning project files.
- `ls_tree` and `ls_files` keep `include_hidden` as a separate switch for dot-paths; `.gitignore` filters are still applied.
- `pull` with explicit `file_paths` skips paths ignored by `<root_dir>/.gitignore`.

## Acceptance checks
- `make lint`
- `make test`
- `make e2e-blackbox`
