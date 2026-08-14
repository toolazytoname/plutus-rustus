#!/usr/bin/env bash
# Stop the supervisor and engine started by start.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SUPERVISOR_PID="$ROOT/data/supervisor.pid"
ENGINE_PID="$ROOT/data/engine.pid"

stop_pidfile() {
  local file="$1"
  if [[ -f "$file" ]]; then
    local pid
    pid="$(cat "$file")"
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      for _ in 1 2 3 4 5; do
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
      done
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$file"
  fi
}

stop_pidfile "$SUPERVISOR_PID"
stop_pidfile "$ENGINE_PID"
echo "stopped"
