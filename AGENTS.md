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

## Zero-Twin Verification Policy (RFC-0270)

Never write "mock" or "twin" implementations for verification tools.
1. **Loom:** Do NOT create mock structs like `LoomWriteGroup`. Instead, route production concurrency primitives through `crate::sync_kernel` and write `loom::model` tests that invoke the REAL production structs and methods.
2. **Stateright:** Stateright models must execute the actual production `fn`s for state transitions. Re-implementing logic inside the model is strictly forbidden.
3. **Verus:** If Verus does not support a type, use `verus_keep_ghost` to add ghost state/proofs without altering the production executable types, or isolate the logic into pure `no_std` kernels.
4. **Anti-Vacuity (RFC-0329):** If a proof or test passes, explicitly ask "Would it fail if I injected a bug?". Verification without mutation testing is vacuous. All continuous DST campaigns must execute and kill the full M1..M7 anti-vacuity battery (`rfc0325_dst_campaign`) to prove that mechanical oracles have teeth before any trial is declared clean.

## Absolute Rigor Policy — Prohibition of Premature 100% Claims (RFC-0273 / RFC-0329)

Never claim verification or testing completion (EG2/EG3 "100%") while any of the following 5 weaknesses exist:
1. **Mutation Score < 98%:** All core kernels must be fuzzed by `scripts/mutation_fuzzer.py` and zero-recompile mutation switching (RFC-0329), killing >= 98% of synthetic AST mutants.
2. **DST In-Memory Only:** Never claim DST proof solely on `mem_storage=true`. Official claims require `PEDRA_SWARM_DISK=1` running on real POSIX filesystems with `pwrite`/`fdatasync`.
3. **M2 Composition < 80%:** Never claim end-to-end formal proof when composition chaining is low. M2 chaining must reach >= 80% (265+/331 atomic functions).
4. **Uncontracted Glue:** Handlers in `pedradb-posix` and `pedradb-io-uring` must be guarded by contracts in `pedradb-spec`.
5. **Combinatorial Loom Overclaiming:** Never claim Loom verified the full database. Loom is strictly for isolated atomic primitives (<= 3 threads). Full engine concurrency must be proven via PCT (Probabilistic Concurrency Testing).

