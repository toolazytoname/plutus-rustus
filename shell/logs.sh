#!/usr/bin/env bash
# Tail the engine log.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="$ROOT/logs/goldpan.log"
if [[ ! -f "$LOG" && -f "$ROOT/logs/plutus.log" ]]; then
  LOG="$ROOT/logs/plutus.log"
fi
mkdir -p "$ROOT/logs"
touch "$LOG"
exec tail -n 100 -f "$LOG"
