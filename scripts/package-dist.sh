#!/usr/bin/env bash
# Pack a portable release tree: bin/ + operator scripts, no git history.
# usage: scripts/package-dist.sh <binary> <os> <arch> <out-dir>
set -euo pipefail

BIN="${1:?binary}"
OS="${2:?os}"
ARCH="${3:?arch}"
OUT="${4:?out-dir}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="goldpan-${OS}-${ARCH}"

[[ -f "$BIN" && -x "$BIN" ]] || {
  echo "not an executable: $BIN" >&2
  exit 1
}

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
PREFIX="${STAGE}/${NAME}"
mkdir -p "$PREFIX/bin" "$PREFIX/shell" "$PREFIX/deploy" "$PREFIX/docs"

cp "$BIN" "$PREFIX/bin/goldpan"
chmod +x "$PREFIX/bin/goldpan"
if command -v strip >/dev/null 2>&1; then
  strip "$PREFIX/bin/goldpan" 2>/dev/null || true
fi

cp "$ROOT/config.example.toml" "$PREFIX/config.example.toml"
cp "$ROOT/env.example" "$PREFIX/env.example"
cp "$ROOT/install.sh" "$PREFIX/install.sh"
cp "$ROOT/README.md" "$PREFIX/README.md"
cp "$ROOT/docs/DEPLOY.md" "$PREFIX/docs/DEPLOY.md"
chmod +x "$PREFIX/install.sh"

for f in plutus start.sh stop.sh status.sh logs.sh install-systemd.sh update.sh install_start.sh common.sh; do
  cp "$ROOT/shell/$f" "$PREFIX/shell/$f"
  chmod +x "$PREFIX/shell/$f"
done

cp "$ROOT/deploy/goldpan.service" "$PREFIX/deploy/goldpan.service"
cp "$ROOT/deploy/goldpan-update.service" "$PREFIX/deploy/goldpan-update.service"
cp "$ROOT/deploy/goldpan-update.timer" "$PREFIX/deploy/goldpan-update.timer"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
tar -C "$STAGE" -czf "${OUT}/${NAME}.tar.gz" "$NAME"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUT" && sha256sum "${NAME}.tar.gz" >"${NAME}.tar.gz.sha256")
else
  (cd "$OUT" && shasum -a 256 "${NAME}.tar.gz" >"${NAME}.tar.gz.sha256")
fi

echo "wrote ${OUT}/${NAME}.tar.gz"
ls -lh "${OUT}/${NAME}.tar.gz" "${OUT}/${NAME}.tar.gz.sha256"
