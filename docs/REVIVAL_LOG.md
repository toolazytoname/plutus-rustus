# Revival Log

## 2026-08-13 — mmap lookup: Bloom + on-disk buckets (~85MB)

- Default `lookup = "mmap"`: Bloom + 64K-bucket index in RAM, hash160 table on
  disk (`PLH2`). False negatives are tests; exact check is a `pread` + binary
  search of one first-2-byte bucket.
- Pickle import and gzip refresh use chunked external sort (~20MB/chunk) so
  building the snapshot no longer holds 44M keys in RAM.
- Legacy `PLH1` converts in a streaming two-pass. `posix_fadvise(DONTNEED)`
  keeps the OS page cache from retaining the whole table.

## 2026-08-13 — ops: Bark, CPU cap, daily snapshot, weak-host profile

- Notifications default to Bark; daily "still alive" heartbeat; `notify-test`.
- `engine.profile = low|balanced|full`, `cpu_percent`, and mmap lookup.
- Stale snapshots recycle workers, drop RAM, download, then reload.
- Operator scripts and `docs/DEPLOY.md`. Git history was audited: live
  ServerChan/dufs secrets were never committed.

## 2026-08-13 — collider productized for long-running full-set search

- Default coverage now includes uncompressed P2PKH as well as compressed
  P2PKH/P2WPKH, with SIMD hash160 for both encodings.
- Added x86_64 SHA-NI + 4-wide SSE2 hash160 (crate fallback without SHA-NI).
- Runtime database is a versioned binary `addresses.h160` snapshot; pickles
  migrate automatically. `data update` refreshes from the Loyce dump.
- Engine writes fsynced findings, `data/status.json`, and handles SIGINT/SIGTERM.
  Notifications never include private keys.
- Replaced helper scripts that contained credentials with env-only supervisor
  scripts and systemd units under `deploy/`.

## 2026-08-11 — baseline prepared

- Restored the GitHub repository from archived state.
- Added and fully fetched `upstream` (`a137x/plutus-rustus`).
- Created `codex/revive-plutus` from `upstream/master` at `d7caeb3`.
- Initialised `depend/secp256k1` at its pinned submodule revision.
- Added the recovery plan and implementation handoff checklist.
- Added `plutus-watch`, a watch-only local TSV snapshot monitor with optional
  generic-webhook and ServerChan status notifications.
- Added a policy-compatible GitHub Actions workflow and verified formatting,
  strict Clippy, and all deterministic tests locally.

Next: complete P0 repository hygiene and establish a passing, fixture-only CI
baseline before adding the snapshot and watch-only modules.
