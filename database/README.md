# Database FAQ

This database is a list of Bitcoin addresses that currently hold a positive balance,
serialized into several `.pickle` files. Two address types are kept — **P2PKH**
(`1...`) and native SegWit **P2WPKH** (`bc1q...`) — because both encode
`hash160(compressed pubkey)`, which is exactly what the collider generates. A single
generated `hash160` is therefore checked against both types in one lookup, at no
extra cost in the hot loop.

### Source

The address set comes from [Loyce Club](http://addresses.loyce.club/)
(`Bitcoin_addresses_LATEST.txt.gz`), which republishes [Blockchair](https://blockchair.com/)'s
daily dump of every Bitcoin address with a balance. This is the same widely-used
source that current Plutus forks rely on; it replaces the original
`btcposbal2csv` method, which required running a full node and is effectively defunct.

Only P2PKH (`1...`) and P2WPKH (`bc1q...`, bech32 v0, 20-byte program) addresses are
kept — both are `hash160(pubkey)`. `3...` (P2SH), P2WSH, and Taproot (`bc1p...`)
addresses use a different payload the collider can never produce, so they are dropped
during preparation. This keeps the database smaller and the load fast.

### Format

At startup the engine prefers `data/addresses.h160`, a versioned binary snapshot
(`PLH2`: Bloom bits + 64K-bucket index + sorted 20-byte `hash160` records).
Older `PLH1` files convert in place on first load. The first run migrates the
pickle slices automatically; later starts skip pickle decode. Refresh with:

```bash
./target/release/plutus-rustus data update
# or from an existing pickle tree:
./target/release/plutus-rustus data prepare
```

The pickle slices remain a portable source format: Python `list[str]`, pickle
protocol 4, up to `1,000,000` addresses per file. The folder name is the snapshot
date in `MON_DD_YYYY` format.

### How Many Addresses Does The Database Have?

The current snapshot (`JUL_12_2026`) holds **`44,365,067` funded addresses**:

| type | prefix | count |
|---|---|---:|
| P2PKH | `1...` | 21,273,320 |
| P2WPKH | `bc1q...` | 23,091,747 |

Note that funded **bech32 (P2WPKH) addresses now outnumber legacy P2PKH** — earlier
snapshots kept only P2PKH and discarded all of these, so including them roughly
doubles the reachable set at zero cost in the hot loop (both decode to the same
`hash160(pubkey)`). Excluded from the dump: `3...` (P2SH), longer `bc1q...` (P2WSH),
and `bc1p...` (Taproot), none of which a P2PKH-style generator can match.

### How To Refresh The Database

```bash
# 1. Download the latest funded-address list (~1.4 GB gz)
curl -O http://addresses.loyce.club/Bitcoin_addresses_LATEST.txt.gz

# 2. Extract P2PKH ("1...") and P2WPKH ("bc1q...") addresses (streamed).
#    make_pickles.py refines these (e.g. drops longer bc1q P2WSH), and the Rust
#    loader re-validates every address by fully decoding it.
gunzip -c Bitcoin_addresses_LATEST.txt.gz | grep -E '^(1|bc1q)' > funded.txt

# 3. Re-chunk into pickle slices into a new dated folder, then update
#    DB_VER in src/main.rs to match the folder name.
python3 make_pickles.py funded.txt database/MON_DD_YYYY
./target/release/plutus-rustus data prepare
```

`data update` does the download + filter + binary snapshot in one step.
