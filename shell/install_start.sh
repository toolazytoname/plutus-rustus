#!/usr/bin/env bash
# First-time local install: config, dirs, native build, doctor.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f config.toml ]]; then
  cp config.example.toml config.toml
  echo "wrote config.toml from config.example.toml (profile=low)"
fi
if [[ ! -f .env ]]; then
  cp env.example .env
  echo "wrote .env — put your Bark device key in PLUTUS_BARK_KEY"
fi

mkdir -p data findings logs
if [[ ! -x target/release/plutus-rustus ]]; then
  echo "building release binary with native CPU flags"
  cargo rustc --release -- -C target-cpu=native
fi

# shellcheck disable=SC1091
set -a
source "$ROOT/.env"
set +a
export PLUTUS_CONFIG="${PLUTUS_CONFIG:-$ROOT/config.toml}"
"$ROOT/target/release/plutus-rustus" doctor
echo
echo "next:"
echo "  1. edit .env  (PLUTUS_BARK_KEY, PLUTUS_NODE_NAME)"
echo "  2. ./target/release/plutus-rustus notify-test"
echo "  3. bash shell/start.sh"
echo "docs: docs/DEPLOY.md"
