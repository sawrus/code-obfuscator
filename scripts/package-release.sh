#!/usr/bin/env bash
set -euo pipefail

APP="code-obfuscator"
TARGET="${1:-}"
BINARY_PATH="${2:-}"
OUTPUT_DIR="${3:-dist}"

if [[ -z "$TARGET" || -z "$BINARY_PATH" ]]; then
  echo "usage: scripts/package-release.sh <target-triple> <binary-path> [output-dir]" >&2
  exit 1
fi

case "$TARGET" in
  x86_64-unknown-linux-gnu) artifact="${APP}-linux-x64.tar.gz" ; inner_name="$APP" ;;
  aarch64-unknown-linux-gnu) artifact="${APP}-linux-arm64.tar.gz" ; inner_name="$APP" ;;
  x86_64-apple-darwin) artifact="${APP}-darwin-x64.tar.gz" ; inner_name="$APP" ;;
  aarch64-apple-darwin) artifact="${APP}-darwin-arm64.tar.gz" ; inner_name="$APP" ;;
  x86_64-pc-windows-gnu|x86_64-pc-windows-msvc) artifact="${APP}-windows-x64.zip" ; inner_name="${APP}.exe" ;;
  aarch64-pc-windows-msvc) artifact="${APP}-windows-arm64.zip" ; inner_name="${APP}.exe" ;;
  *)
    echo "unsupported target: $TARGET" >&2
    exit 1
    ;;
esac

mkdir -p "$OUTPUT_DIR"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/${APP}.package.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
cp "$BINARY_PATH" "$tmp_dir/$inner_name"
chmod 755 "$tmp_dir/$inner_name" || true

case "$artifact" in
  *.tar.gz)
    tar -C "$tmp_dir" -czf "$OUTPUT_DIR/$artifact" "$inner_name"
    ;;
  *.zip)
    if command -v zip >/dev/null 2>&1; then
      (cd "$tmp_dir" && zip -q "$OLDPWD/$OUTPUT_DIR/$artifact" "$inner_name")
    else
      echo "zip is required to build $artifact" >&2
      exit 1
    fi
    ;;
esac

echo "$OUTPUT_DIR/$artifact"
