#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! -f config/config.toml ]]; then
  cp config/config.example.toml config/config.toml
  echo "created config/config.toml from config.example.toml"
fi

exec cargo run -p fastcart-server -- "$@"
