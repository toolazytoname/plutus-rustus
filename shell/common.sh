# Shared by operator scripts. ROOT must be set before sourcing.
plutus_resolve_bin() {
  local root="${1:-${ROOT:-.}}"
  if [[ -n "${PLUTUS_BIN:-}" && -x "$PLUTUS_BIN" ]]; then
    printf '%s\n' "$PLUTUS_BIN"
    return 0
  fi
  if [[ -x "$root/bin/plutus-rustus" ]]; then
    printf '%s\n' "$root/bin/plutus-rustus"
    return 0
  fi
  if [[ -x "$root/target/release/plutus-rustus" ]]; then
    printf '%s\n' "$root/target/release/plutus-rustus"
    return 0
  fi
  return 1
}
