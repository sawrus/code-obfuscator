# MCP Git-Style Tool Cheatsheet

## Legacy -> New Mapping

1. `list_project_tree` -> `ls_tree`
2. `obfuscate_project` / `obfuscate_project_from_paths` -> `pull` (or `clone` for full workspace)
3. `apply_llm_output` -> `push`
4. `deobfuscate_*` -> `push` (request-scoped, workspace diff based)

## Shared Argument Rules

1. Keep one `options.request_id` for all calls in a round-trip.
2. Use normalized paths, no parent traversal.
3. `workspace_dir` for `status`/`push` must match the workspace bound by `clone`.

## `ls_tree`

### Request
```json
{
  "root_dir": "/workspace/projects/test",
  "max_depth": 4,
  "max_entries": 200,
  "include_hidden": false
}
```

### Response Shape
```json
{
  "root_dir": "/workspace/projects/test",
  "entries": [{"path": "query.py", "kind": "file"}],
  "truncated": false
}
```

## `ls_files`

### Request
```json
{
  "root_dir": "/workspace/projects/test",
  "max_entries": 200,
  "include_hidden": false
}
```

### Response Shape
```json
{
  "root_dir": "/workspace/projects/test",
  "files": ["query.py"],
  "truncated": false
}
```

## `pull`

### Request (subset)
```json
{
  "root_dir": "/workspace/projects/test",
  "file_paths": ["query.py"],
  "options": {
    "request_id": "req-001"
  }
}
```

### Response Shape
```json
{
  "request_id": "req-001",
  "obfuscated_files": [{"path": "query.py", "content": "..."}],
  "stats": {"file_count": 1, "mapping_entries": 1},
  "events": [{"stage": "completed", "timestamp_epoch_s": 0}]
}
```

## `clone`

### Request (full workspace)
```json
{
  "root_dir": "/workspace/projects/test",
  "workspace_dir": "/workspace/projects/test-obf",
  "options": {
    "request_id": "req-001"
  }
}
```

### Response Shape
```json
{
  "request_id": "req-001",
  "workspace_dir": "/workspace/projects/test-obf",
  "cloned_files": ["query.py"],
  "stats": {"file_count": 1, "mapping_entries": 1},
  "events": [{"stage": "cloned", "timestamp_epoch_s": 0}]
}
```

## `status`

### Request
```json
{
  "workspace_dir": "/workspace/projects/test-obf",
  "options": {
    "request_id": "req-001"
  }
}
```

### Response Shape
```json
{
  "request_id": "req-001",
  "workspace_dir": "/workspace/projects/test-obf",
  "clean": false,
  "diff": {
    "added": ["new.py"],
    "modified": ["query.py"],
    "deleted": ["old.py"]
  },
  "mapping_state": {
    "mapping_entries": 1,
    "tracked_files": 3,
    "root_dir": "/workspace/projects/test",
    "workspace_dir": "/workspace/projects/test-obf"
  }
}
```

## `push`

### Request
```json
{
  "workspace_dir": "/workspace/projects/test-obf",
  "options": {
    "request_id": "req-001"
  }
}
```

### Response Shape
```json
{
  "request_id": "req-001",
  "applied_files": ["query.py", "new.py"],
  "deleted_files": ["old.py"],
  "stats": {
    "applied_count": 2,
    "deleted_count": 1,
    "mapping_entries": 1
  },
  "events": [{"stage": "completed", "timestamp_epoch_s": 0}]
}
```

## Golden Round-Trip Sequence

1. `ls_tree` and/or `ls_files`
2. `clone`
3. edit obfuscated workspace files
4. `status` (expect `clean=false`)
5. `push`
6. `status` (expect `clean=true`)

## Common Errors and Fixes

1. `unknown request_id`:
   - Cause: stale/mismatched `request_id`.
   - Fix: restart flow from `clone` with a new `request_id`.

2. `root_dir does not exist`:
   - Cause: wrong runtime mount path.
   - Fix: check MCP server volume mapping and runtime path.

3. `root_dir is not writable for push`:
   - Cause: source root mounted read-only.
   - Fix: mount project volume as read-write for MCP server.

4. `refusing to write through symlink`:
   - Cause: symlink guardrail.
   - Fix: replace symlink with regular file in source root.
