#!/usr/bin/env bash
set -euo pipefail

IMAGE_NAME="code-obfuscator-mcp:local"
CONTAINER_NAME="${MCP_CONTAINER_NAME:-code-obfuscator-mcp}"
MCP_HTTP_ADDR="${MCP_HTTP_ADDR:-}"
MAPPING_FILE="${MCP_DEFAULT_MAPPING_PATH:-}"
PROJECTS_HOST_DIR="${MCP_PROJECTS_HOST_DIR:-}"
SKIP_BUILD="${MCP_SKIP_DOCKER_BUILD:-false}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

if [[ ! "$SKIP_BUILD" =~ ^(1|true|yes|on)$ ]]; then
  docker build -f docker/mcp.Dockerfile -t "$IMAGE_NAME" .
fi

echo "Starting MCP server from Docker image: $IMAGE_NAME"

DOCKER_ARGS=(run --rm --name "$CONTAINER_NAME")

if [[ -n "$MCP_HTTP_ADDR" ]]; then
  # expecting format host:port, publish same port on host
  PORT="${MCP_HTTP_ADDR##*:}"
  DOCKER_ARGS+=( -e "MCP_HTTP_ADDR=0.0.0.0:${PORT}" -e "MCP_DISABLE_STDIO=true" -p "${PORT}:${PORT}" )
else
  DOCKER_ARGS+=( -i )
fi

if [[ -n "$MAPPING_FILE" ]]; then
  ABS_MAPPING="$(cd "$(dirname "$MAPPING_FILE")" && pwd)/$(basename "$MAPPING_FILE")"
  DOCKER_ARGS+=( -e "MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json" -v "${ABS_MAPPING}:/data/mapping.default.json" )
fi

if [[ -n "$PROJECTS_HOST_DIR" ]]; then
  if [[ ! -d "$PROJECTS_HOST_DIR" ]]; then
    echo "projects dir not found: $PROJECTS_HOST_DIR" >&2
    exit 1
  fi
  ABS_PROJECTS="$(cd "$PROJECTS_HOST_DIR" && pwd)"
  DOCKER_ARGS+=( -v "${ABS_PROJECTS}:/workspace/projects:rw" )
fi

for env_name in MCP_LOG_STDOUT MCP_LOG_MODE MCP_LOG_DIR MCP_LOG_MAX_BYTES MCP_LOG_MAX_FILES; do
  if [[ -n "${!env_name:-}" ]]; then
    DOCKER_ARGS+=( -e "${env_name}=${!env_name}" )
  fi
done

DOCKER_ARGS+=("$IMAGE_NAME")
exec docker "${DOCKER_ARGS[@]}"
