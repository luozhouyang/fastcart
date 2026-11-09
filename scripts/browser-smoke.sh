#!/usr/bin/env bash
# Browser-level smoke test for the cart/checkout deep links.
#
# Prerequisite: a running Fastcart server with a store whose default theme is
# active and whose `cart`/`checkout` pages were seeded (provision the store once
# after upgrading). The script drives headless Chrome because the repository
# does not ship Playwright.
set -euo pipefail

BASE_URL="${FASTCART_BASE_URL:-http://shop.example.com:8080}"
STORE_HOST="${FASTCART_HOST:-shop.example.com}"
CHROME_BIN="${CHROME_BIN:-google-chrome}"
RESOLVE_IP="${FASTCART_RESOLVE_IP:-127.0.0.1}"

if ! command -v "$CHROME_BIN" >/dev/null 2>&1; then
  echo "browser-smoke: $CHROME_BIN not found" >&2
  exit 1
fi

profile="$(mktemp -d)"
trap 'rm -rf "$profile"' EXIT

for path in /cart /checkout; do
  dom="$(mktemp)"
  "$CHROME_BIN" \
    --headless=new \
    --no-sandbox \
    --disable-gpu \
    --disable-dev-shm-usage \
    --disable-extensions \
    --no-proxy-server \
    --user-data-dir="$profile" \
    --host-resolver-rules="MAP $STORE_HOST $RESOLVE_IP" \
    --virtual-time-budget=5000 \
    --dump-dom "$BASE_URL$path" >"$dom" 2>/dev/null

  if grep -q "Page not found" "$dom"; then
    echo "browser-smoke: $path rendered the theme's not-found page" >&2
    exit 1
  fi
  if ! grep -q 'role="dialog"' "$dom"; then
    echo "browser-smoke: $path did not open the cart drawer after hydration" >&2
    exit 1
  fi
  if ! grep -q 'aria-expanded="true"' "$dom"; then
    echo "browser-smoke: $path left the cart launcher collapsed" >&2
    exit 1
  fi
  rm -f "$dom"
  echo "browser-smoke: $path opened the cart drawer"
done
