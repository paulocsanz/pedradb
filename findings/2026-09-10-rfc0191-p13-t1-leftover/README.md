# RFC-0191 P1.3 — T1 leftover `model→atom`

`leftover_txn_is_aborted()` was a constant `true` (`leftover_txn_is_aborted_true`
without ∀). Product T1 does not move on that.

## Kernel

`leftover_fate(committed: bool) -> bool` = `!committed` in
`crates/pedradb-store/src/txn_kernel.rs`. As-is always `false` (leftover
materialises). `leftover_txn_is_aborted()` is now `leftover_fate(false)`.

`abort_leftover_intents` matches per-tid: classify `committed` from
`txn_status_key`, skip when `!leftover_fate(committed)`.

## Proof

`t1_leftover_fate` : ∀ (committed : Bool), leftover_fate committed = ok (!committed)
in `Txn.lean`. Extract: `aeneas_store_txn.sh` + `aeneas_txn.sh` + `aeneas_t1_modelo.sh`.

Product TSV T1 `model→atom`, floor_promoted 2→3. Depth-floor extract=276
close=1 atom=1 unchanged (no new catalog pair — would grow data_fate past
cap 130). The recovered `if` lived in store, not the db.rs trampoline.

## Named test

`leftover_fate_on_live_committed_is_not_ok` in txn_kernel.rs. Isolated crate
`formal/aeneas/store-txn-kernel` (does not need pedradb-core). Full
`pedradb-store` compile is red this host: otimizar made `Db::mem` private
(`concurrent.rs:3387`).
