#!/usr/bin/env bash
# Installs the pinned workerd binary used by the Phase 0 fixtures.
# The binary is large and is intentionally not committed.
set -euo pipefail

VERSION="v1.20261109.1"
ASSET="workerd-linux-64.gz"
SHA256="543f30be7dc9b6e263226fa08daa86cbb505b4b407048fbcdb82d0f1b910ad80"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPERIMENT="$REPO_ROOT/experiments/workerd"
DEST="$EXPERIMENT/bin/workerd"
URL="https://github.com/cloudflare/workerd/releases/download/${VERSION}/${ASSET}"

if [[ -x "$DEST" ]]; then
  current="$("$DEST" --version 2>/dev/null || true)"
  if [[ "$current" == "workerd 2026-11-09" ]]; then
    echo "workerd already installed: $DEST ($current)"
    exit 0
  fi
fi

mkdir -p "$EXPERIMENT/bin"
tmp="$(mktemp "$EXPERIMENT/bin/workerd.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

echo "downloading workerd ${VERSION}..."
curl -fL --retry 3 --connect-timeout 20 "$URL" | gunzip >"$tmp"
echo "${SHA256}  ${tmp}" | sha256sum -c -
chmod +x "$tmp"
mv "$tmp" "$DEST"
trap - EXIT

"$DEST" --version
echo "installed: $DEST"
