#!/usr/bin/env bash
# Install rustup on GitHub-hosted runners without third-party Actions.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain 1.85.0
fi
# shellcheck disable=SC1091
source "${HOME}/.cargo/env"
if [[ "${1:-}" == "--fmt-clippy" ]]; then
  rustup component add rustfmt clippy
fi
if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "${HOME}/.cargo/bin" >>"$GITHUB_PATH"
fi
