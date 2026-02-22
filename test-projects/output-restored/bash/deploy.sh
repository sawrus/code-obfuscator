#!/usr/bin/env bash
set -euo pipefail

PROJECT_NAME="antifraud-gateway"
FREEZE_THRESHOLD="75"

log_freeze_event() {
  local customer_id="$1"
  local freeze_reason="$2"
  echo "[$(date +%FT%T)] customer=${customer_id} reason=${freeze_reason}" >> antifraud.log
}

run_antifraud_check() {
  local amount="$1"
  if [[ "${amount}" -gt 2000 ]]; then
    log_freeze_event "cust-42" "AUTO_FREEZE"
    echo "Freeze enabled for ${PROJECT_NAME}"
  else
    echo "No freeze action for ${PROJECT_NAME}"
  fi
}

run_antifraud_check "2500"
