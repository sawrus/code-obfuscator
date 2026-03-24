#!/usr/bin/env bash
set -euo pipefail

command -v codex >/dev/null 2>&1 || {
  echo "codex CLI not found in PATH" >&2
  exit 1
}

codex mcp remove code_obfuscator >/dev/null 2>&1 || true
MCP_TRANSPORT="${MCP_TRANSPORT:-stdio}"

case "$MCP_TRANSPORT" in
  stdio)
    PROJECTS_HOST_DIR="${MCP_PROJECTS_HOST_DIR:-$HOME/projects}"
    CONTAINER_NAME="${CONTAINER_NAME:-code-obfuscator-mcp}"
    codex mcp add code_obfuscator -- \
      docker run --rm -i --name "$CONTAINER_NAME" \
      -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
      -e MCP_LOG_STDOUT=false \
      -v "$HOME/mcp/code-obfuscator/mapping.default.json:/data/mapping.default.json:ro" \
      -v "$PROJECTS_HOST_DIR:/workspace/projects:rw" \
      code-obfuscator-mcp:local
    ;;
  http)
    MCP_HTTP_URL="${MCP_HTTP_URL:?MCP_HTTP_URL is required for MCP_TRANSPORT=http}"
    codex mcp add code_obfuscator --url "$MCP_HTTP_URL"
    ;;
  *)
    echo "unsupported MCP_TRANSPORT: $MCP_TRANSPORT" >&2
    exit 1
    ;;
esac
