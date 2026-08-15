# Database

The collider does **not** ship funded addresses in git. On a machine, download
and build the snapshot with:

```bash
~/plutus-rustus/shell/plutus update-db
# same as:
~/plutus-rustus/bin/plutus-rustus data update
```

That pulls [Loyce Club](http://addresses.loyce.club/)'s
`Bitcoin_addresses_LATEST.txt.gz` (Blockchair's daily dump of addresses with a
balance), keeps only **P2PKH** (`1...`) and **P2WPKH** (`bc1q...`), and writes
`data/addresses.h160` (`PLH2`: Bloom + 64K-bucket index + sorted 20-byte records).

Those two types both encode `hash160(compressed pubkey)`, which is exactly what
the generator produces. P2SH (`3...`), P2WSH, and Taproot (`bc1p...`) use a
different payload and are dropped.

A recent dump had **44,365,067** funded hash160s (21.3M P2PKH + 23.1M P2WPKH).
The gzip is about 1.4 GB; it is streamed and never stored in this repository.

Optional pickle slices (`database/MON_DD_YYYY/*.pickle`) are still accepted by
`data prepare` if you have a local tree. They are gitignored and must not be
committed.
