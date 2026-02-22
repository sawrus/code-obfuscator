#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "Usage: $0 <source_dir> <obfuscated_dir> <restored_dir> [mapping_json]" >&2
  exit 1
fi

SOURCE_DIR="$1"
OBF_DIR="$2"
RESTORED_DIR="$3"
MAPPING_JSON="${4:-}"

PROJECT_DIR="/Users/isaev/projects/ai/codex/upstream-code-obfuscator"
BIN="$PROJECT_DIR/target/debug/code-obfuscator"

if [[ ! -d "$SOURCE_DIR" ]]; then
  echo "Source directory not found: $SOURCE_DIR" >&2
  exit 1
fi

if [[ ! -x "$BIN" ]]; then
  (cd "$PROJECT_DIR" && cargo build >/dev/null)
fi

FORWARD_ARGS=(--mode forward --source "$SOURCE_DIR" --target "$OBF_DIR")
if [[ -n "$MAPPING_JSON" ]]; then
  FORWARD_ARGS+=(--mapping "$MAPPING_JSON")
fi

"$BIN" "${FORWARD_ARGS[@]}"

GEN_MAPPING="$OBF_DIR/mapping.generated.json"
if [[ ! -f "$GEN_MAPPING" ]]; then
  echo "Generated mapping not found: $GEN_MAPPING" >&2
  exit 1
fi

"$BIN" --mode reverse --source "$OBF_DIR" --target "$RESTORED_DIR" --mapping "$GEN_MAPPING"

diff -ru "$SOURCE_DIR" "$RESTORED_DIR" >/dev/null

echo "Round-trip succeeded: restored tree matches source."
