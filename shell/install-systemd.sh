#!/usr/bin/env bash
# Install systemd units with this checkout as WorkingDirectory.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="${UNIT_DIR:-/etc/systemd/system}"
INSTALL_ROOT="${PLUTUS_ROOT:-$ROOT}"
CPU_QUOTA="${PLUTUS_CPU_QUOTA:-40%}"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "re-run with sudo: sudo bash $0" >&2
  exit 1
fi

sed \
  -e "s|/opt/plutus-rustus|$INSTALL_ROOT|g" \
  -e "s|^CPUQuota=.*|CPUQuota=$CPU_QUOTA|" \
  "$ROOT/deploy/plutus.service" >"$UNIT_DIR/plutus.service"

sed "s|/opt/plutus-rustus|$INSTALL_ROOT|g" \
  "$ROOT/deploy/plutus-update.service" >"$UNIT_DIR/plutus-update.service"
cp "$ROOT/deploy/plutus-update.timer" "$UNIT_DIR/plutus-update.timer"

systemctl daemon-reload
systemctl enable plutus.service
echo "installed $UNIT_DIR/plutus.service (CPUQuota=$CPU_QUOTA, MemoryMax=512M)"
echo "start: systemctl start plutus"
echo "logs:  journalctl -u plutus -f"
echo "leave plutus-update.timer disabled; the engine refreshes the snapshot itself"
