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

Product floor (RFC-0041, re-baselined 2026-08-24 — registered product
decision): the official gate is `ROCKS_PARITY_RATIO_FLOOR=1.0` on the
drop-in same-class column (async Pedra vs that default peer; 15/15 shapes
≥ 1.254, `findings/rocks-parity-floor1x/`). The G1 product column
(fdatasync before Ok) is the published per-shape claim table
(`findings/rocks-parity-floor1x-g1/`): reads 1.128–1.986× default with
stronger durability; single-client write-per-op shapes are fd-ceiling
below 1× by construction (one full barrier per op vs the peer's zero;
group commit closes them under concurrency — apply_mc4 2.788× head3).
Never quote those rows as wins; never hide them. Sync-peer ratios are
not a win.
