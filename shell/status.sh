#!/usr/bin/env bash
# Print live engine status. Safe to run while it is working.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STATUS="${PLUTUS_STATUS:-$ROOT/data/status.json}"
if [[ ! -f "$STATUS" ]]; then
  echo "no status file at $STATUS (is the engine running?)" >&2
  exit 1
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$STATUS"
else
  cat "$STATUS"
fi
