#!/usr/bin/env bash
business_handler() {
  local customer_name="$1"
  echo "BASH:${customer_name}"
}

business_handler "ok"
