#!/usr/bin/env bash
# One-command FastCart plugin demo.
#
# Starts a disposable API + admin, creates a demo store and owner, packs the
# chat example and installs it through the real admin API. Everything lives in
# .fastcart-demo/ unless FASTCART_DEMO_DIR (or --dir) says otherwise.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${FASTCART_DEMO_DIR:-$ROOT/.fastcart-demo}"
API_PORT="${FASTCART_DEMO_API_PORT:-18080}"
ADMIN_PORT="${FASTCART_DEMO_ADMIN_PORT:-15174}"
EMAIL="${FASTCART_DEMO_EMAIL:-demo@fastcart.local}"
PASSWORD="${FASTCART_DEMO_PASSWORD:-fastcart-demo}"
STORE_SLUG="demo"
STORE_DOMAIN="shop.example.com"
WORKERD="${FASTCART_WORKERD:-}"
CLEAN=0
BUILD=1
INSTALL_DEPS=1
PLUGIN=1
OPEN=1

usage() {
  cat <<'USAGE'
Usage: scripts/plugin-demo.sh [options]

  --dir <path>       Demo data directory (default: .fastcart-demo)
  --api-port <port>  API port (default: 18080)
  --admin-port <port> Admin port (default: 15174)
  --clean            Delete the demo directory first
  --no-build         Skip cargo build
  --no-install       Skip pnpm install
  --no-plugin        Start without workerd/chat plugin
  --no-open          Do not open the browser
  -h, --help

Environment:
  FASTCART_DEMO_DIR, FASTCART_DEMO_API_PORT, FASTCART_DEMO_ADMIN_PORT,
  FASTCART_DEMO_EMAIL, FASTCART_DEMO_PASSWORD, FASTCART_WORKERD
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir) DEMO_DIR="$2"; shift 2 ;;
    --api-port) API_PORT="$2"; shift 2 ;;
    --admin-port) ADMIN_PORT="$2"; shift 2 ;;
    --clean) CLEAN=1; shift ;;
    --no-build) BUILD=0; shift ;;
    --no-install) INSTALL_DEPS=0; shift ;;
    --no-plugin) PLUGIN=0; shift ;;
    --no-open) OPEN=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "plugin-demo: unknown option $1" >&2; usage >&2; exit 2 ;;
  esac
done

log() { printf '[plugin-demo] %s\n' "$*"; }
fail() { printf '[plugin-demo] error: %s\n' "$*" >&2; exit 1; }

for tool in cargo pnpm node curl awk; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing $tool"
done

case "$DEMO_DIR" in
  "$ROOT"/*) ;;
  *) fail "demo directory must be inside $ROOT (got $DEMO_DIR)" ;;
esac
DEMO_DIR="$(cd "$(dirname "$DEMO_DIR")" 2>/dev/null && pwd)/$(basename "$DEMO_DIR")" || DEMO_DIR="$DEMO_DIR"

if [[ -n "$WORKERD" ]]; then
  [[ -x "$WORKERD" ]] || fail "FASTCART_WORKERD is not executable: $WORKERD"
elif [[ -x "$ROOT/experiments/workerd/bin/workerd" ]]; then
  WORKERD="$ROOT/experiments/workerd/bin/workerd"
elif command -v workerd >/dev/null 2>&1; then
  WORKERD="$(command -v workerd)"
fi

if [[ "$PLUGIN" -eq 1 && -z "$WORKERD" ]]; then
  fail "workerd is required for --plugin; run scripts/install-workerd.sh or set FASTCART_WORKERD"
fi

if [[ "$CLEAN" -eq 1 ]]; then
  log "resetting $DEMO_DIR"
  rm -rf "$DEMO_DIR"
fi
mkdir -p "$DEMO_DIR/data" "$DEMO_DIR/artifacts"

CONFIG="$DEMO_DIR/config.toml"
COOKIE="$DEMO_DIR/cookies.txt"

cat >"$CONFIG" <<TOML
[server]
host = "127.0.0.1"
port = $API_PORT
cookie_secure = false
cors_origins = ["http://localhost:$ADMIN_PORT"]

[plugins]
enabled = $([[ "$PLUGIN" -eq 1 ]] && echo true || echo false)
root = "./data/plugins"
runtime_dir = "./data/plugins/runtime"
self_address = "127.0.0.1:$API_PORT"
$([[ -n "$WORKERD" ]] && printf 'workerd_binary = "%s"\n' "$WORKERD" || true)
TOML

if [[ "$INSTALL_DEPS" -eq 1 && ! -d "$ROOT/node_modules" ]]; then
  log "installing JavaScript dependencies"
  (cd "$ROOT" && pnpm install --frozen-lockfile)
fi

if [[ "$BUILD" -eq 1 ]]; then
  log "building fastcart CLI (first run can take a while)"
  (cd "$ROOT" && cargo build -p fastcart-cli)
fi

BIN="$ROOT/target/debug/fastcart"
[[ -x "$BIN" ]] || fail "missing $BIN; run without --no-build"

cd "$DEMO_DIR"
export FASTCART_CONFIG="$CONFIG"
export FASTCART_SERVER_HOST="127.0.0.1"
export FASTCART_SERVER_PORT="$API_PORT"
export FASTCART_PLUGINS_ENABLED="$([[ "$PLUGIN" -eq 1 ]] && echo true || echo false)"
export FASTCART_PLUGIN_WORKERD_BINARY="$WORKERD"
export FASTCART_PLUGIN_SELF_ADDRESS="127.0.0.1:$API_PORT"

log "creating demo owner (existing owner is left unchanged)"
"$BIN" user create --email "$EMAIL" --password "$PASSWORD" --first-name Demo >/dev/null 2>"$DEMO_DIR/user.log" || true

if "$BIN" store list 2>/dev/null | awk -F '\t' -v slug="$STORE_SLUG" '$2 == slug { found=1 } END { exit !found }'; then
  log "reusing store $STORE_SLUG"
else
  log "creating store $STORE_SLUG"
  "$BIN" store create --name "Demo Store" --slug "$STORE_SLUG" --domain "$STORE_DOMAIN" --owner-email "$EMAIL"
fi
"$BIN" user grant --email "$EMAIL" --store-id "$("$BIN" store list | awk -F '\t' -v slug="$STORE_SLUG" '$2 == slug { print $1; exit }')" --role owner >/dev/null 2>"$DEMO_DIR/grant.log" || true
STORE_ID="$("$BIN" store list | awk -F '\t' -v slug="$STORE_SLUG" '$2 == slug { print $1; exit }')"
[[ -n "$STORE_ID" ]] || fail "could not resolve the demo store id"

ARCHIVE=""
if [[ "$PLUGIN" -eq 1 ]]; then
  log "packing the chat example"
  node --experimental-strip-types "$ROOT/packages/plugin-cli/src/cli.ts" pack "$ROOT/templates/plugin-chat" --out "$DEMO_DIR/artifacts" >/dev/null
  ARCHIVE="$(find "$DEMO_DIR/artifacts" -name 'chat-widget-*.tar.gz' | sort | tail -1)"
  [[ -n "$ARCHIVE" ]] || fail "chat plugin archive was not produced"
fi

log "starting API on http://127.0.0.1:$API_PORT"
"$BIN" serve --host 127.0.0.1 --port "$API_PORT" >"$DEMO_DIR/server.log" 2>&1 &
SERVER_PID=$!
ADMIN_PID=""

kill_tree() {
  local pid="$1"
  [[ -n "$pid" ]] || return 0
  local child
  while read -r child; do
    [[ -n "$child" ]] && kill_tree "$child"
  done < <(pgrep -P "$pid" 2>/dev/null || true)
  kill "$pid" 2>/dev/null || true
}

cleanup() {
  trap - EXIT INT TERM
  kill_tree "$ADMIN_PID"
  kill_tree "$SERVER_PID"
  [[ -n "$ADMIN_PID" ]] && wait "$ADMIN_PID" 2>/dev/null || true
  [[ -n "$SERVER_PID" ]] && wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 300); do
  if curl -fsS "http://127.0.0.1:$API_PORT/health" >/dev/null 2>&1; then break; fi
  kill -0 "$SERVER_PID" 2>/dev/null || { tail -40 "$DEMO_DIR/server.log" >&2; fail "API exited during startup"; }
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$API_PORT/health" >/dev/null 2>&1 || { tail -40 "$DEMO_DIR/server.log" >&2; fail "API did not become healthy"; }

API="http://127.0.0.1:$API_PORT"
LOGIN_CODE="$(curl -sS -o "$DEMO_DIR/login.json" -w '%{http_code}' -c "$COOKIE" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" \
  "$API/api/v1/platform/auth/login")"
[[ "$LOGIN_CODE" == "200" ]] || { cat "$DEMO_DIR/login.json" >&2; fail "login failed with HTTP $LOGIN_CODE (try --clean)"; }
CSRF="$(awk '$6 == "fc_csrf" { print $7 }' "$COOKIE" | tail -1)"
[[ -n "$CSRF" ]] || fail "login did not set the fc_csrf cookie"

if [[ "$PLUGIN" -eq 1 ]]; then
  log "installing and enabling chat-widget"
  EXISTING="$(curl -sS -b "$COOKIE" -H "x-store-id: $STORE_ID" "$API/api/v1/admin/plugins" || true)"
  if grep -q '"id":"chat-widget"' <<<"$EXISTING"; then
    log "chat-widget is already installed"
  else
    INSTALL_CODE="$(curl -sS -o "$DEMO_DIR/install.json" -w '%{http_code}' -b "$COOKIE" \
      -H "x-store-id: $STORE_ID" -H "x-csrf-token: $CSRF" \
      -F "archive=@$ARCHIVE" "$API/api/v1/admin/plugins/install")"
    [[ "$INSTALL_CODE" == "201" ]] || { cat "$DEMO_DIR/install.json" >&2; fail "plugin install failed with HTTP $INSTALL_CODE"; }
  fi
  curl -sS -o /dev/null -b "$COOKIE" \
    -H "x-store-id: $STORE_ID" -H "x-csrf-token: $CSRF" \
    -X POST "$API/api/v1/admin/plugins/chat-widget/enable" || true
  ENABLED="$(curl -sS -b "$COOKIE" -H "x-store-id: $STORE_ID" "$API/api/v1/admin/plugins/chat-widget")"
  if ! grep -q '"enabled":true' <<<"$ENABLED"; then
    printf '%s\n' "$ENABLED" >&2
    fail "chat-widget was installed but not enabled"
  fi
fi

log "starting admin on http://localhost:$ADMIN_PORT"
(cd "$ROOT" && VITE_FASTCART_API_URL="$API" pnpm --filter @fastcart/admin exec vite --host 127.0.0.1 --port "$ADMIN_PORT" >"$DEMO_DIR/admin.log" 2>&1) &
ADMIN_PID=$!

for _ in $(seq 1 300); do
  if curl -fsS "http://127.0.0.1:$ADMIN_PORT/" >/dev/null 2>&1; then break; fi
  kill -0 "$ADMIN_PID" 2>/dev/null || { tail -40 "$DEMO_DIR/admin.log" >&2; fail "admin exited during startup"; }
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$ADMIN_PORT/" >/dev/null 2>&1 || { tail -40 "$DEMO_DIR/admin.log" >&2; fail "admin did not become ready"; }

ADMIN_URL="http://localhost:$ADMIN_PORT"
cat <<INFO

  FastCart plugin demo is ready.

  Admin:    $ADMIN_URL
  API:      $API
  Store:    $STORE_SLUG ($STORE_ID)
  Login:    $EMAIL / $PASSWORD
  Plugin:   $([[ "$PLUGIN" -eq 1 ]] && echo 'chat-widget (open Plugins)' || echo 'not installed')
  Logs:     $DEMO_DIR/{server,admin}.log

  Press Ctrl-C to stop both processes.

INFO

if [[ "$OPEN" -eq 1 ]]; then
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$ADMIN_URL" >/dev/null 2>&1 || true
  elif command -v open >/dev/null 2>&1; then
    open "$ADMIN_URL" >/dev/null 2>&1 || true
  fi
fi

while kill -0 "$SERVER_PID" 2>/dev/null && kill -0 "$ADMIN_PID" 2>/dev/null; do
  sleep 1
done
log "a process exited; stopping the demo"
