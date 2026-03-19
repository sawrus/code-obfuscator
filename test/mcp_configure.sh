#!/usr/bin/env bash
set -euo pipefail

command -v codex >/dev/null 2>&1 || {
  echo "codex CLI not found in PATH" >&2
  exit 1
}

codex mcp remove code_obfuscator >/dev/null 2>&1 || true
codex mcp add code_obfuscator -- \
  docker run --rm -i \
  -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
  -e MCP_LOG_STDOUT=false \
  -v $HOME/mcp/code-obfuscator/mapping.default.json:/data/mapping.default.json:ro \
  -v $HOME/projects:/workspace/projects:rw \
  code-obfuscator-mcp:local
