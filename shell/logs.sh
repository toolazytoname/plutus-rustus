#!/usr/bin/env bash
# Tail the engine log.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="$ROOT/logs/plutus.log"
mkdir -p "$ROOT/logs"
touch "$LOG"
exec tail -n 100 -f "$LOG"
