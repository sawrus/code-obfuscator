#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXTRACT_PROMPT_TEMPLATE="$SCRIPT_DIR/prompt_extract_sql.txt"
EXTRACT_SOURCE_FILE="$SCRIPT_DIR/query_api_x.py"
EXTRACT_EXPECTED_FILE="$SCRIPT_DIR/query_api_x_expected.txt"
MAPPING_DIR="$HOME/mcp/code-obfuscator"
MAPPING_FILE="$MAPPING_DIR/mapping.default.json"

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "required command not found: $cmd" >&2
    exit 1
  }
}

require_fixture() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "required fixture is missing: $path" >&2
    exit 1
  fi
}

run_codex_prompt() {
  local prompt_file="$1"
  local output_file="$2"
  local last_message_file="$3"

  cd "$REPO_ROOT"
  if ! codex exec --sandbox workspace-write -o "$last_message_file" < "$prompt_file" 2>&1 | tee "$output_file"; then
    echo "codex exec failed; full output:" >&2
    cat "$output_file" >&2
    return 1
  fi
}

assert_tool_call_present() {
  local output_file="$1"
  local tool="$2"
  if ! grep -q "code_obfuscator\\.${tool}(" "$output_file"; then
    echo "missing MCP tool call in output: $tool" >&2
    cat "$output_file" >&2
    return 1
  fi
}

assert_any_tool_call_present() {
  local output_file="$1"
  shift
  local tool

  for tool in "$@"; do
    if grep -q "code_obfuscator\\.${tool}(" "$output_file"; then
      return 0
    fi
  done

  echo "missing expected discovery MCP tool call in output: $*" >&2
  cat "$output_file" >&2
  return 1
}

assert_tool_call_absent() {
  local output_file="$1"
  local tool="$2"
  if grep -q "code_obfuscator\\.${tool}(" "$output_file"; then
    echo "unexpected MCP tool call in output: $tool" >&2
    cat "$output_file" >&2
    return 1
  fi
}

assert_no_tool_failures() {
  local output_file="$1"
  if grep -E -q "mcp startup: failed|code_obfuscator\\.[a-z_]+\\(.*\\) failed" "$output_file"; then
    echo "MCP tool call failed during blackbox run" >&2
    cat "$output_file" >&2
    return 1
  fi
}

assert_text_file_equals() {
  local expected_file="$1"
  local actual_file="$2"
  local label="$3"
  local expected_text
  local actual_text

  expected_text="$(cat "$expected_file")"
  actual_text="$(cat "$actual_file")"
  if [[ "$actual_text" != "$expected_text" ]]; then
    echo "unexpected ${label}" >&2
    echo "--- expected ---" >&2
    printf '%s\n' "$expected_text" >&2
    echo "--- actual ---" >&2
    printf '%s\n' "$actual_text" >&2
    return 1
  fi
}

wait_for_http_ready() {
  local port="$1"
  local attempts=30

  for ((i = 1; i <= attempts; i++)); do
    if curl -fsS "http://127.0.0.1:${port}/health" >/dev/null; then
      return 0
    fi
    sleep 1
  done

  echo "HTTP MCP server did not become ready on port ${port}" >&2
  return 1
}

start_http_server() {
  local projects_host_dir="$1"
  local container_name="$2"
  local log_file="$3"
  local http_port="$4"

  docker rm -f "$container_name" >/dev/null 2>&1 || true
  if ! docker run -d -i --rm \
    --name "$container_name" \
    -e "MCP_HTTP_ADDR=0.0.0.0:${http_port}" \
    -e MCP_DEFAULT_MAPPING_PATH=/data/mapping.default.json \
    -e MCP_LOG_STDOUT=false \
    -v "$MAPPING_FILE:/data/mapping.default.json:ro" \
    -v "$projects_host_dir:/workspace/projects:rw" \
    -p "${http_port}:${http_port}" \
    code-obfuscator-mcp:local >/dev/null; then
    docker logs "$container_name" >"$log_file" 2>&1 || true
    cat "$log_file" >&2
    return 1
  fi

  if ! wait_for_http_ready "$http_port"; then
    docker logs "$container_name" >"$log_file" 2>&1 || true
    cat "$log_file" >&2
    return 1
  fi
}

cleanup_scenario() {
  local status="$1"
  local tmpdir="$2"
  local container_name="$3"

  if [[ -n "$container_name" ]]; then
    docker rm -f "$container_name" >/dev/null 2>&1 || true
  fi
  codex mcp remove code_obfuscator >/dev/null 2>&1 || true

  if [[ "$status" -eq 0 ]]; then
    rm -rf "$tmpdir"
  else
    echo "debug artifacts kept at: $tmpdir" >&2
  fi
}

scenario_extract_sql() {
  local tmpdir="$1"
  local transport="$2"
  local container_name="code-obfuscator-mcp-${transport}-$$"
  SCENARIO_CONTAINER_NAME="$container_name"

  local output_file="$tmpdir/codex-output.txt"
  local last_message_file="$tmpdir/codex-last-message.txt"
  local prompt_file="$tmpdir/prompt.runtime.txt"
  local http_log_file="$tmpdir/http-mcp.log"
  local http_port="${MCP_HTTP_TEST_PORT:-$((20000 + RANDOM % 20000))}"
  local projects_host_dir="$tmpdir/projects"
  local project_dir="$projects_host_dir/team-a/backend/api-x-api"
  local workspace_dir="$projects_host_dir/team-a/backend/api-x-api-obf"
  local mcp_workspace_dir="/workspace/projects/team-a/backend/api-x-api-obf"

  mkdir -p "$project_dir" "$projects_host_dir/team-a/backend/another-service"
  mkdir -p "$MAPPING_DIR"

  cp "$EXTRACT_SOURCE_FILE" "$project_dir/query.py"
  printf '{}\n' > "$MAPPING_FILE"

  sed \
    -e "s|{{MCP_WORKSPACE_DIR}}|$mcp_workspace_dir|g" \
    -e "s|{{HOST_WORKSPACE_DIR}}|$workspace_dir|g" \
    "$EXTRACT_PROMPT_TEMPLATE" > "$prompt_file"

  if [[ "$transport" == "stdio" ]]; then
    MCP_TRANSPORT="stdio" \
    MCP_PROJECTS_HOST_DIR="$projects_host_dir" \
    CONTAINER_NAME="$container_name" \
      bash "$SCRIPT_DIR/mcp_configure.sh"
  elif [[ "$transport" == "http" ]]; then
    start_http_server "$projects_host_dir" "$container_name" "$http_log_file" "$http_port"
    MCP_TRANSPORT="http" \
    MCP_HTTP_URL="http://127.0.0.1:${http_port}/mcp" \
      bash "$SCRIPT_DIR/mcp_configure.sh"
  else
    echo "unsupported extract transport: $transport" >&2
    return 1
  fi

  run_codex_prompt "$prompt_file" "$output_file" "$last_message_file"

  assert_any_tool_call_present "$output_file" "ls_tree" "ls_files"
  assert_tool_call_present "$output_file" "clone"
  assert_tool_call_absent "$output_file" "push"
  assert_no_tool_failures "$output_file"
  if ! grep -q 'find-api-x-sql-01' "$output_file"; then
    echo "expected request_id was not observed in blackbox output" >&2
    cat "$output_file" >&2
    return 1
  fi

  assert_text_file_equals "$EXTRACT_EXPECTED_FILE" "$last_message_file" "SQL extraction output"

  echo "blackbox ok: SQL extraction over ${transport} matched expected result"
}

run_scenario() {
  local name="$1"
  local func="$2"
  shift 2
  local tmpdir
  local local_status

  tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/code-obfuscator-${name}.XXXXXX")"
  echo "running blackbox scenario: ${name}"
  SCENARIO_CONTAINER_NAME=""
  set +e
  (
    set -euo pipefail
    "$func" "$tmpdir" "$@"
  )
  local_status=$?
  set -e
  cleanup_scenario "$local_status" "$tmpdir" "$SCENARIO_CONTAINER_NAME"
  return "$local_status"
}

main() {
  require_cmd make
  require_cmd docker
  require_cmd codex
  require_cmd curl
  require_fixture "$EXTRACT_PROMPT_TEMPLATE"
  require_fixture "$EXTRACT_SOURCE_FILE"
  require_fixture "$EXTRACT_EXPECTED_FILE"

  mkdir -p "$MAPPING_DIR"

  cd "$REPO_ROOT"
  make mcp-docker-build

  run_scenario "extract-sql-stdio" scenario_extract_sql "stdio"
  run_scenario "extract-sql-http" scenario_extract_sql "http"
}

main "$@"
exit 0
