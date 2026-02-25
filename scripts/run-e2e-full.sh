#!/usr/bin/env bash
set -euo pipefail

IMAGE_NAME="code-obfuscator-e2e"

docker build -f docker/e2e.Dockerfile -t "${IMAGE_NAME}" .
docker run --rm -v "$(pwd)":/work -w /work "${IMAGE_NAME}" bash -lc 'cargo test --tests -- --nocapture'
