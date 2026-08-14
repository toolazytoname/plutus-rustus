#!/usr/bin/env bash
# Manual snapshot refresh. With auto_update=true the engine already does this
# after dropping RAM; use this script when auto_update is false, or to force it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

BIN="${PLUTUS_BIN:-$ROOT/target/release/plutus-rustus}"
export PLUTUS_CONFIG="${PLUTUS_CONFIG:-$ROOT/config.toml}"

"$BIN" data update
if [[ "${1:-}" != "--no-restart" ]]; then
  "$ROOT/shell/stop.sh"
  "$ROOT/shell/start.sh"
fi
