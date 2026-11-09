#!/usr/bin/env bash
# Phase 0 workerd acceptance tests. Requires a pinned workerd binary.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKERD="${WORKERD:-$ROOT/bin/workerd}"
LOG_DIR="${TMPDIR:-/tmp}/fastcart-workerd-phase0"
mkdir -p "$LOG_DIR"
PIDS=()

cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

if [[ ! -x "$WORKERD" ]]; then
  echo "workerd binary not found at $WORKERD" >&2
  echo "run scripts/install-workerd.sh first" >&2
  exit 1
fi

wait_for_port() {
  local port="$1"
  for _ in $(seq 1 100); do
    if curl -fsS --max-time 1 "http://127.0.0.1:${port}/" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_worker() {
  local name="$1"
  local config="$2"
  local port="$3"
  "$WORKERD" serve "$config" >"$LOG_DIR/${name}.log" 2>&1 &
  local pid=$!
  PIDS+=("$pid")
  if ! wait_for_port "$port"; then
    echo "workerd $name did not become ready; log:" >&2
    cat "$LOG_DIR/${name}.log" >&2
    exit 1
  fi
  echo "  $name ready on $port"
}

assert_json() {
  local name="$1"
  local json="$2"
  local expression="$3"
  node -e "const d = JSON.parse(process.argv[1]); if (!($expression)) { console.error('$name assertion failed:', JSON.stringify(d)); process.exit(1) }" "$json"
  echo "  $name ok"
}

stop_worker() {
  local pid="${PIDS[-1]}"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  unset 'PIDS[-1]'
}

echo "basic Worker"
start_worker basic "$ROOT/configs/basic.capnp" 18787
body="$(curl -fsS "http://127.0.0.1:18787/products/example?q=1")"
assert_json basic "$body" "d.message === 'FastCart SSR Runtime' && d.path === '/products/example' && d.method === 'GET'"
stop_worker

echo "standard Web APIs"
start_worker web-api "$ROOT/configs/web-api.capnp" 18788
body="$(curl -fsS "http://127.0.0.1:18788/")"
assert_json web-api "$body" \
  "Object.values(d.apis).every(Boolean) && d.request.method === 'POST' && d.request.header === 'yes, again' && d.streamed === 'stream-works' && d.sha256.length === 64 && d.timerElapsedMs >= 15 && d.abortObserved === true"
stop_worker

echo "streaming response"
start_worker streaming "$ROOT/configs/streaming.capnp" 18789
stream_file="$LOG_DIR/stream.txt"
rm -f "$stream_file"
curl -fsSN "http://127.0.0.1:18789/" | while IFS= read -r line; do
  printf '%s %s\n' "$(date +%s%3N)" "$line" >>"$stream_file"
done
lines="$(wc -l <"$stream_file")"
if [[ "$lines" -ne 3 ]]; then
  echo "expected 3 stream chunks, got $lines" >&2
  cat "$stream_file" >&2
  exit 1
fi
first_ms="$(head -1 "$stream_file" | awk '{print $1}')"
last_ms="$(tail -1 "$stream_file" | awk '{print $1}')"
elapsed_ms=$((last_ms - first_ms))
if [[ "$elapsed_ms" -lt 500 ]]; then
  echo "streaming was buffered: first and last chunk were ${elapsed_ms}ms apart" >&2
  cat "$stream_file" >&2
  exit 1
fi
echo "  streaming ok (${elapsed_ms}ms between chunks)"
stop_worker

echo "service bindings"
start_worker service-bindings "$ROOT/configs/service-bindings.capnp" 18790
body="$(curl -fsS "http://127.0.0.1:18790/")"
assert_json service-bindings "$body" "d.from === 'app' && d.status === 200 && d.api.from === 'api' && d.api.storeId === 'store_123'"
stop_worker

echo "Phase 0 workerd checks passed"
