#!/usr/bin/env bash
set -euo pipefail

IMAGE_NAME="code-obfuscator-mcp:local"
MCP_HTTP_ADDR="${MCP_HTTP_ADDR:-}"
MAPPING_FILE="${MCP_DEFAULT_MAPPING_PATH:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

docker build -f docker/mcp.Dockerfile -t "$IMAGE_NAME" .

echo "Starting MCP server from Docker image: $IMAGE_NAME"

DOCKER_ARGS=(run --rm -i)

if [[ -n "$MCP_HTTP_ADDR" ]]; then
  # expecting format host:port, publish same port on host
  PORT="${MCP_HTTP_ADDR##*:}"
  DOCKER_ARGS+=( -e "MCP_HTTP_ADDR=0.0.0.0:${PORT}" -p "${PORT}:${PORT}" )
fi

if [[ -n "$MAPPING_FILE" ]]; then
  ABS_MAPPING="$(cd "$(dirname "$MAPPING_FILE")" && pwd)/$(basename "$MAPPING_FILE")"
  DOCKER_ARGS+=( -e "MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json" -v "${ABS_MAPPING}:/data/mapping.default.json" )
fi

DOCKER_ARGS+=("$IMAGE_NAME")
exec docker "${DOCKER_ARGS[@]}"
