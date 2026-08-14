#!/usr/bin/env bash
# Example local file host. Authentication belongs in the environment, not here.
set -euo pipefail

AUTH="${DUFS_AUTH:-}"
if [[ -z "$AUTH" ]]; then
  echo "set DUFS_AUTH=user:pass before running" >&2
  exit 1
fi

docker run --rm -v "$(pwd)":/data -p 5000:5000 sigoden/dufs /data -A -a "${AUTH}@/:rw"
