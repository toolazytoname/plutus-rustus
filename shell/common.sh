# Shared by operator scripts. ROOT must be set before sourcing.
# Deployed process name is goldpan; cargo still builds plutus-rustus.
plutus_resolve_bin() {
  local root="${1:-${ROOT:-.}}"
  if [[ -n "${PLUTUS_BIN:-}" && -x "$PLUTUS_BIN" ]]; then
    printf '%s\n' "$PLUTUS_BIN"
    return 0
  fi
  local candidate
  for candidate in \
    "$root/bin/goldpan" \
    "$root/target/release/plutus-rustus" \
    "$root/bin/plutus-rustus"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

plutus_install_bin() {
  local src="${1:?}" dest_dir="${2:?}"
  mkdir -p "$dest_dir"
  cp "$src" "$dest_dir/goldpan"
  chmod +x "$dest_dir/goldpan"
}
