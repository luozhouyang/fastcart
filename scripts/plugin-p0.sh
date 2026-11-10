#!/usr/bin/env bash
# Runs the plugin runtime P0 validation suite.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGINS="$ROOT/experiments/workerd/plugins"
WORKERD="$ROOT/experiments/workerd/bin/workerd"

if [[ ! -x "$WORKERD" ]]; then
  echo "workerd not installed; run scripts/install-workerd.sh first" >&2
  exit 2
fi

export FASTCART_WORKERD_BIN="$WORKERD"
export FASTCART_WORKERD_EXPERIMENTAL="${FASTCART_WORKERD_EXPERIMENTAL:-1}"

rm -f "$PLUGINS/report/results.jsonl"
(cd "$PLUGINS" && node --test --test-concurrency=1 suites/*.test.mjs)
node "$PLUGINS/harness/report.mjs"

