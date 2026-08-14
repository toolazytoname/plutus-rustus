#!/usr/bin/env bash
# Optional helper for copying this tree to an operator-owned file host.
# Keep host URLs, basic-auth, and tokens out of the repository.
set -euo pipefail

if [[ -z "${PLUTUS_UPLOAD_URL:-}" ]]; then
  echo "set PLUTUS_UPLOAD_URL to your file host, e.g. https://files.example/plutus/" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
curl ${PLUTUS_UPLOAD_AUTH:+-u "$PLUTUS_UPLOAD_AUTH"} -T "$ROOT/shell/start.sh" "$PLUTUS_UPLOAD_URL/start.sh"
