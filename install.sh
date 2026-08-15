#!/usr/bin/env bash
# curl | bash installer. Downloads a CI-built binary; default profile is low.
#
#   curl -fsSL https://raw.githubusercontent.com/toolazytoname/plutus-rustus/main/install.sh | bash -s --
#   curl -fsSL .../install.sh | bash -s -- --profile=full --fetch-db --start
#   PLUTUS_BARK_KEY=xxx curl -fsSL ... | bash -s -- --start
#
# Needs curl + tar. No git, no compiler, no Rust.
set -euo pipefail

if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "use bash: curl -fsSL <url>/install.sh | bash -s -- [--profile=low]" >&2
  exit 1
fi

GITHUB="${PLUTUS_GITHUB:-toolazytoname/plutus-rustus}"
VERSION="${PLUTUS_VERSION:-latest}"
DEST="${PLUTUS_HOME:-}"
PROFILE="${PLUTUS_PROFILE:-low}"
PROFILE_SET=0
FETCH_DB=0
DO_START=0
FROM_SOURCE=0

usage() {
  cat <<EOF
usage: install.sh [--profile=low|balanced|full] [--dir PATH] [--version=latest|nightly|vX.Y.Z]
                  [--fetch-db] [--start] [--from-source]

  --profile      low (default, ~75MB) | balanced | full
  --dir          install path (default: ~/plutus-rustus, or /opt/plutus-rustus as root)
  --version      GitHub Release tag (default: latest, then nightly)
  --fetch-db     download the funded-address snapshot now (~1.4GB)
  --start        start the collider after install
  --from-source  clone and cargo-build instead of a Release binary (needs git + Rust)

Bark: set PLUTUS_BARK_KEY in the environment; it is written to .env and never printed.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="$2"
      PROFILE_SET=1
      shift 2
      ;;
    --profile=*)
      PROFILE="${1#*=}"
      PROFILE_SET=1
      shift
      ;;
    --dir)
      DEST="$2"
      shift 2
      ;;
    --dir=*)
      DEST="${1#*=}"
      shift
      ;;
    --version)
      VERSION="$2"
      shift 2
      ;;
    --version=*)
      VERSION="${1#*=}"
      shift
      ;;
    --nightly)
      VERSION=nightly
      shift
      ;;
    --fetch-db) FETCH_DB=1; shift ;;
    --start) DO_START=1; shift ;;
    --from-source) FROM_SOURCE=1; shift ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

case "$PROFILE" in
  low | balanced | full) ;;
  *)
    echo "profile must be low, balanced, or full" >&2
    exit 1
    ;;
esac

need_cmd() {
  command -v "$1" >/dev/null 2>&1
}

detect_artifact() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *)
      echo "unsupported OS: $os" >&2
      exit 1
      ;;
  esac
  case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *)
      echo "unsupported arch: $arch" >&2
      exit 1
      ;;
  esac
  echo "${os}-${arch}"
}

sha256_check() {
  local sums="$1"
  if need_cmd sha256sum; then
    sha256sum -c "$sums"
  elif need_cmd shasum; then
    shasum -a 256 -c "$sums"
  else
    echo "need sha256sum or shasum to verify the download" >&2
    exit 1
  fi
}

download() {
  local dest="$1" url="$2"
  echo "fetching $url"
  curl -fL --retry 3 --retry-delay 1 -o "$dest" "$url"
}

install_binary() {
  local artifact name tmp url base
  artifact="$(detect_artifact)"
  name="plutus-rustus-${artifact}.tar.gz"
  need_cmd curl || {
    echo "need curl" >&2
    exit 1
  }
  need_cmd tar || {
    echo "need tar" >&2
    exit 1
  }

  mkdir -p "$DEST"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  base="https://github.com/${GITHUB}/releases"
  if [[ "$VERSION" == "latest" ]]; then
    if download "$tmp/$name" "${base}/latest/download/${name}"; then
      url="${base}/latest/download/${name}"
    else
      echo "no stable release yet, trying nightly"
      VERSION=nightly
      download "$tmp/$name" "${base}/download/nightly/${name}"
      url="${base}/download/nightly/${name}"
    fi
  else
    download "$tmp/$name" "${base}/download/${VERSION}/${name}"
    url="${base}/download/${VERSION}/${name}"
  fi
  download "$tmp/${name}.sha256" "${url}.sha256"
  (cd "$tmp" && sha256_check "${name}.sha256")

  tar -xzf "$tmp/$name" -C "$tmp"
  local tree="$tmp/plutus-rustus-${artifact}"
  [[ -d "$tree" ]] || {
    echo "tarball had no payload at $tree" >&2
    exit 1
  }

  mkdir -p "$DEST/bin" "$DEST/shell" "$DEST/deploy" "$DEST/docs" "$DEST/data" "$DEST/findings" "$DEST/logs"
  cp "$tree/bin/plutus-rustus" "$DEST/bin/plutus-rustus"
  chmod +x "$DEST/bin/plutus-rustus"
  cp -R "$tree/shell/." "$DEST/shell/"
  chmod +x "$DEST/shell/"*
  cp -R "$tree/deploy/." "$DEST/deploy/"
  cp "$tree/config.example.toml" "$DEST/config.example.toml"
  cp "$tree/env.example" "$DEST/env.example"
  cp "$tree/docs/DEPLOY.md" "$DEST/docs/DEPLOY.md" 2>/dev/null || true
  cp "$tree/install.sh" "$DEST/install.sh"
  chmod +x "$DEST/install.sh" "$DEST/shell/plutus"
  echo "installed $DEST/bin/plutus-rustus ($artifact, $VERSION)"
}

ensure_tools() {
  if need_cmd git && need_cmd curl && { need_cmd cc || need_cmd gcc || need_cmd clang; }; then
    return 0
  fi
  if need_cmd apt-get; then
    echo "installing git, curl, and a C compiler"
    if [[ "$(id -u)" -eq 0 ]]; then
      apt-get update -qq
      DEBIAN_FRONTEND=noninteractive apt-get install -y git curl build-essential pkg-config
    elif need_cmd sudo; then
      sudo apt-get update -qq
      sudo DEBIAN_FRONTEND=noninteractive apt-get install -y git curl build-essential pkg-config
    else
      echo "need git, curl, and a C compiler (apt install git curl build-essential)" >&2
      exit 1
    fi
    return 0
  fi
  echo "need git, curl, and a C compiler on PATH" >&2
  exit 1
}

ensure_rust() {
  export PATH="${HOME}/.cargo/bin:${PATH}"
  if need_cmd cargo; then
    return 0
  fi
  echo "installing rustup (stable)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
}

in_tree_root() {
  local here=""
  if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    if [[ -f "$here/Cargo.toml" && -d "$here/src" ]]; then
      echo "$here"
      return 0
    fi
  fi
  return 1
}

install_from_source() {
  local repo="${PLUTUS_REPO:-https://github.com/${GITHUB}.git}"
  local branch="${PLUTUS_BRANCH:-main}"
  local root
  root="$(in_tree_root || true)"
  if [[ -z "$root" ]]; then
    ensure_tools
    ensure_rust
    if [[ -d "$DEST/.git" ]]; then
      echo "updating $DEST"
      git -C "$DEST" fetch origin
      git -C "$DEST" checkout "$branch"
      git -C "$DEST" pull --ff-only origin "$branch"
      git -C "$DEST" submodule update --init --recursive
    else
      echo "cloning $repo ($branch) -> $DEST"
      git clone --recursive --branch "$branch" "$repo" "$DEST"
    fi
    extra=()
    [[ "$FETCH_DB" -eq 1 ]] && extra+=(--fetch-db)
    [[ "$DO_START" -eq 1 ]] && extra+=(--start)
    [[ "$PROFILE_SET" -eq 1 ]] && extra+=(--profile="$PROFILE")
    extra+=(--from-source)
    exec bash "$DEST/install.sh" --dir="$DEST" "${extra[@]}"
  fi
  DEST="${DEST:-$root}"
  ensure_rust
}

resolve_dest() {
  if [[ -n "$DEST" ]]; then
    return 0
  fi
  local root
  root="$(in_tree_root || true)"
  if [[ -n "$root" ]]; then
    DEST="$root"
  elif [[ "$(id -u)" -eq 0 ]]; then
    DEST=/opt/plutus-rustus
  else
    DEST="$HOME/plutus-rustus"
  fi
}

resolve_dest
if [[ "$FROM_SOURCE" -eq 1 ]]; then
  install_from_source
else
  install_binary
fi

export PLUTUS_CPU_QUOTA
case "$PROFILE" in
  low) PLUTUS_CPU_QUOTA="${PLUTUS_CPU_QUOTA:-40%}" ;;
  balanced) PLUTUS_CPU_QUOTA="${PLUTUS_CPU_QUOTA:-70%}" ;;
  full) PLUTUS_CPU_QUOTA="${PLUTUS_CPU_QUOTA:-100%}" ;;
esac

args=(install)
if [[ "$PROFILE_SET" -eq 1 ]]; then
  args+=(--profile="$PROFILE")
elif [[ ! -f "$DEST/config.toml" ]]; then
  args+=(--profile="$PROFILE")
fi
[[ "$FETCH_DB" -eq 1 ]] && args+=(--fetch-db)
[[ "$DO_START" -eq 1 ]] && args+=(--start)
[[ "$FROM_SOURCE" -eq 1 ]] && args+=(--from-source)
exec "$DEST/shell/plutus" "${args[@]}"
