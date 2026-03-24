# MCP Server Config Examples

## Codex CLI

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

## Claude MCP Connector (conceptual)

Use the equivalent connector UI/JSON fields:
1. server name: `code_obfuscator`
2. command: `docker`
3. args:
   - `run --rm -i`
   - env vars for mapping/logging
   - volume mounts for mapping (ro) and projects (rw)
   - image `code-obfuscator-mcp:local`
4. allowlist tools:
   - `ls_tree`, `ls_files`, `pull`, `clone`, `status`, `push`

## Gemini MCP Bridge (conceptual)

For Gemini environments that use MCP bridge adapters:
1. register server command identical to Codex/Claude command.
2. pass through full JSON args including nested `options.request_id`.
3. ensure project mount is read-write for `push`.
4. if bridge has tool filtering, allow only git-style tools.

## Runtime Mount Pattern

Recommended mount convention:
1. host: `/abs/path/projects`
2. runtime in MCP: `/workspace/projects`
3. source root in prompts: `/workspace/projects/<project-name>`
4. host mirror for shell edits: `/abs/path/projects/<project-name>-obf`

## Validation Smoke Script

Before first real task, run this logical sequence in your IDE:
1. `tools/list`
2. `ls_tree` on project root
3. `clone` with test `request_id`
4. edit one obfuscated file in host mirror
5. `status` -> expect `clean=false`
6. `push`
7. `status` -> expect `clean=true`
