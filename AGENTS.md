# Agent rules (this repo)

## RocksDB parity — official peer (do not re-litigate)

The **only** number that counts as beating Rocks is Pedra vs **RocksDB default**:
`WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`).

Pedra still `fdatasync`s before Ok. That is the product: more durability
**and** faster than the Rocks people actually run. Do not:

- call a win vs `sync=true` “we beat Rocks”
- say “different durability class so it doesn’t count”
- default scripts or compare to a sync peer
- lead tables with the sync column

`rocks-parity-compare` **exits 2** if the peer JSON has `sync: true`
(unless `ROCKS_PARITY_ALLOW_SYNC_PEER=1`).

Product floor (RFC-0041): every official shape ≥ **2×** that default peer
(`compat_qps / rocks_default_qps ≥ 2.0`). Sync-peer ratios are not a win.
