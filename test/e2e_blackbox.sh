#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKDIR="$HOME/projects/test"
INPUT_FILE="$WORKDIR/query.py"
EXPECTED_FILE="$SCRIPT_DIR/query.py"
PROMPT_FILE="$SCRIPT_DIR/prompt.txt"
MAPPING_DIR="$HOME/mcp/code-obfuscator"
MAPPING_FILE="$MAPPING_DIR/mapping.default.json"

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "required command not found: $cmd" >&2
    exit 1
  }
}

require_cmd make
require_cmd docker
require_cmd codex

if [[ ! -f "$EXPECTED_FILE" ]]; then
  echo "expected fixture is missing: $EXPECTED_FILE" >&2
  exit 1
fi

TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/code-obfuscator-blackbox.XXXXXX")"
OUTPUT_FILE="$TMPDIR/codex-output.txt"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$TMPDIR"
  else
    echo "debug artifacts kept at: $TMPDIR" >&2
  fi
}
trap cleanup EXIT

mkdir -p "$WORKDIR" "$MAPPING_DIR"

# Stage the blackbox input expected by the prompt.
sed \
  -e 's/mmm\.users/bs.users/g' \
  -e 's/%(mmm_user_ids)s/%(bs_user_ids)s/g' \
  "$EXPECTED_FILE" > "$INPUT_FILE"

# Keep the Docker-mounted mapping file in sync with the blackbox fixture.
cat > "$MAPPING_FILE" <<'EOF'
{"bs":"mmm"}
EOF

cd "$REPO_ROOT"
make mcp-docker-build
bash "$SCRIPT_DIR/mcp_configure.sh"

if ! codex exec < "$PROMPT_FILE" 2>&1 | tee "$OUTPUT_FILE"; then
  echo "codex exec failed; full output:" >&2
  cat "$OUTPUT_FILE" >&2
  exit 1
fi

expected_block="$(cat "$EXPECTED_FILE")"
actual_block="$(
  awk '
    $0 == "QUERY_1 = \"\"\"" {
      capture = 1
      buf = $0 ORS
      next
    }
    capture {
      buf = buf $0 ORS
      if ($0 == "\"\"\"") {
        last = buf
        capture = 0
      }
    }
    END {
      printf "%s", last
    }
  ' "$OUTPUT_FILE"
)"

if [[ "$actual_block" != "$expected_block" ]]; then
  echo "unexpected final query block" >&2
  echo "--- expected ---" >&2
  printf '%s\n' "$expected_block" >&2
  echo "--- actual ---" >&2
  printf '%s\n' "$actual_block" >&2
  echo "--- full codex output ---" >&2
  cat "$OUTPUT_FILE" >&2
  exit 1
fi

echo "blackbox ok: cli output block matched expected result"
