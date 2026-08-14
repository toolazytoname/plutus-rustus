#!/usr/bin/env bash
# First-time host install. Secrets stay in .env; this script never prints them.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
exec "$ROOT/shell/plutus" install "$@"
