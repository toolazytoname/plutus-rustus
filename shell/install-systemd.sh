#!/usr/bin/env bash
# Install systemd units with this checkout as WorkingDirectory.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="${UNIT_DIR:-/etc/systemd/system}"
INSTALL_ROOT="${PLUTUS_ROOT:-$ROOT}"
CPU_QUOTA="${PLUTUS_CPU_QUOTA:-40%}"
RUN_USER="${PLUTUS_USER:-${SUDO_USER:-root}}"
if [[ -z "$RUN_USER" ]]; then
  RUN_USER=root
fi
if [[ "$RUN_USER" == "root" ]]; then
  RUN_GROUP=root
else
  RUN_GROUP="$(id -gn "$RUN_USER")"
fi

if [[ "$(id -u)" -ne 0 ]]; then
  echo "re-run with sudo: sudo bash $0" >&2
  exit 1
fi

render_unit() {
  local src="$1" dest="$2"
  sed \
    -e "s|/opt/plutus-rustus|$INSTALL_ROOT|g" \
    -e "s|__RUN_USER__|$RUN_USER|g" \
    -e "s|__RUN_GROUP__|$RUN_GROUP|g" \
    -e "s|^CPUQuota=.*|CPUQuota=$CPU_QUOTA|" \
    "$src" >"$dest"
}

render_unit "$ROOT/deploy/goldpan.service" "$UNIT_DIR/goldpan.service"
render_unit "$ROOT/deploy/goldpan-update.service" "$UNIT_DIR/goldpan-update.service"
cp "$ROOT/deploy/goldpan-update.timer" "$UNIT_DIR/goldpan-update.timer"

# Drop the old unit name if this host was installed before the rename.
if [[ -f "$UNIT_DIR/plutus.service" ]]; then
  systemctl disable --now plutus.service 2>/dev/null || true
  rm -f "$UNIT_DIR/plutus.service" "$UNIT_DIR/plutus-update.service" "$UNIT_DIR/plutus-update.timer"
fi

systemctl daemon-reload
systemctl enable goldpan.service
echo "installed $UNIT_DIR/goldpan.service (user=$RUN_USER CPUQuota=$CPU_QUOTA, MemoryMax=256M)"
echo "start: systemctl start goldpan"
echo "logs:  journalctl -u goldpan -f"
echo "leave goldpan-update.timer disabled; the engine refreshes the snapshot itself"
