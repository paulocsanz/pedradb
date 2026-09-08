# Script / trampoline (order, not Env)

`db.rs` / `concurrent.rs` stay unextracted (`glue.db_rs_extracted=false`).
Two leftover layers, different payments:

1. **Predicate `if`** still deciding data fate → rank D/C (named kernel the
   handler calls).
2. **Order** of already-proved kernels + Env (append then maybe `sync_data`
   then apply then `Ok`) still written as glue. That order is payable. The
   syscall is not.

## Plan fn (what to land)

A total `fn` that returns the step sequence (or next action). Production
`match`es and does Env. No leftover order `if` in the handler.

Shape already in-tree as **model**, not yet as the live put/group script:

- `d1_modelo_kernel::put_ok` — append → honest sync → ack
- `write_ack_kernel::WriteAckLedger` — `on_append` / `on_barrier` / `on_ack`;
  AS-IS acks before the barrier

Unpaid live scripts: the board `unpaid_script` (`candidates.py`
`GLUE_SCRIPTS`). Calling `*_plan(` pays **script** only. Compose is a
separate board (`unpaid_compose`): Lean must `unfold` that plan **and**
the callee. ORing them hid compose and sent the grind into an SA factory.

Theorem (two possibilities): `need_sync ⇒ Sync before Apply/Ok`;
`sync_failed ⇒ Fence, not Ok`. AS-IS: Apply/Ok before Sync, or publish
when WAL failed.

`WriteAckLedger` **reports** steps; it does not *choose* them. A plan fn
that `commit_ops_with` matches is the slice. Dual-unfold: plan +
`wal_sync_required` / `fence_on_sync_fail` / `may_publish_group`
(`aeneas.md`).

## Not this

- Dump of `db.rs` / `concurrent.rs`.
- Verus of `Env::sync_data` / POSIX `fdatasync`.
- `media_durable_admitted` becoming true (RFC-0078: `fsync` Ok is not media).
- `lock_interleavings_admitted` / `forall_schedules_admitted` becoming true.
