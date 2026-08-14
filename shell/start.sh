#!/usr/bin/env bash
# Long-running supervisor: crash restart, log rotation, optional env file.
# Secrets belong in .env or the process environment, never in this script.
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
if [[ ! -x "$BIN" ]]; then
  echo "missing $BIN" >&2
  echo "build with: cargo rustc --release -- -C target-cpu=native" >&2
  exit 1
fi

mkdir -p "$ROOT/data" "$ROOT/findings" "$ROOT/logs"
export PLUTUS_CONFIG="${PLUTUS_CONFIG:-$ROOT/config.toml}"

SUPERVISOR_PID="$ROOT/data/supervisor.pid"
ENGINE_PID="$ROOT/data/engine.pid"
LOG="$ROOT/logs/plutus.log"

if [[ -f "$SUPERVISOR_PID" ]] && kill -0 "$(cat "$SUPERVISOR_PID")" 2>/dev/null; then
  echo "already running (supervisor pid $(cat "$SUPERVISOR_PID"))" >&2
  exit 1
fi

rotate_logs() {
  [[ -f "$LOG" ]] || return 0
  local size
  size="$(wc -c <"$LOG" | tr -d ' ')"
  if [[ "$size" -gt $((50 * 1024 * 1024)) ]]; then
    mv "$LOG" "$LOG.$(date -u +%Y%m%dT%H%M%S)"
    ls -1t "$ROOT/logs"/plutus.log.* 2>/dev/null | tail -n +8 | xargs rm -f 2>/dev/null || true
  fi
}

run_loop() {
  trap '[[ -n "${child:-}" ]] && kill "$child" 2>/dev/null || true; exit 0' TERM INT
  while true; do
    rotate_logs
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) starting $BIN run" >>"$LOG"
    "$BIN" run >>"$LOG" 2>&1 &
    child=$!
    echo "$child" >"$ENGINE_PID"
    set +e
    wait "$child"
    code=$?
    set -e
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) engine exit $code" >>"$LOG"
    # 0 = clean stop (SIGTERM). 75 = caller should refresh snapshot (auto_update=false).
    if [[ "$code" -eq 0 ]]; then
      exit 0
    fi
    if [[ "$code" -eq 75 ]]; then
      echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) refreshing snapshot" >>"$LOG"
      "$BIN" data update >>"$LOG" 2>&1 || true
    fi
    sleep 3
  done
}

run_loop &
echo $! >"$SUPERVISOR_PID"
echo "started supervisor pid $(cat "$SUPERVISOR_PID"), logging to $LOG"
