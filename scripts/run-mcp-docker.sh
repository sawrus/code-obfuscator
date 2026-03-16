#!/usr/bin/env bash
set -euo pipefail

IMAGE_NAME="code-obfuscator-mcp:local"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

docker build -f docker/mcp.Dockerfile -t "$IMAGE_NAME" .

echo "Starting MCP server from Docker image: $IMAGE_NAME"
exec docker run --rm -i "$IMAGE_NAME"
