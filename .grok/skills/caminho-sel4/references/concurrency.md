# Four concurrency theorems (not one slogan)

Source of truth for the map: `findings/2026-09-07-concurrency-proof-map/`.
Pay the next unpaid item; do not skip the topic.

| Property | Pedra term (production) | Second possibility | Not this |
|---|---|---|---|
| Data race (UB) | Safe Rust AXM + write-lock **client** protocol: `wal_rotate_decision` with `commit_inflight` keeps WAL | idle rotate | dump of `parking_lot` |
| Lost-update / race condition | `occ_conflict` / `group_validate` (ConcurrentDb `validate_occ_batch` / `lone_commit`) | `occ_conflict_as_is_serialized` | type-system only |
| Deadlock | `wait_for_deadlock` (2PL locktab); ConcurrentDb lock-order = inflight vs flush rotate | as-is never sees cycle | LockBud as a theorem |
| Scheduler ∀π | `lock_interleavings_admitted = ok false` (theorem that the claim is refused) | as-is admits | “PCT d=2 = ∀π” |

**N-way:** `group_validate` of N `OccRead`s on one `last_seq` (Lean example with N>2 + as-is serialized member). DST/PCT plants with N writers name those kernels. Campaign ≠ ∀π.

**Scale / complexity:** enrolled `scale_kernel` theorems on a concrete N; RFC-0176 model. Complexity claims need a number (kernel loc, N members, PCT depth) before a theory.

Literature already in-tree: VerusBelt PLDI 2026 PDF in that findings dir; CapybaraKV OSDI’25 (RW-lock / sharding — same shape as ConcurrentDb); seL4 ticket-lock rely/guarantee (prove **a lock**, not the OS).

## Glue caller is unpaid

Unfolding only the callee does **not** pay the ConcurrentDb / `db.rs` method
that collects facts and sequences I/O. `group_validate_lagging_member_conflicts`
unfolds `group_validate` + `occ_conflict` — it does not pay `validate_occ_batch`.

Pay: a named total fn that method calls (OccRead facts, publish after WAL,
lock-order vs flush), then dual-unfold caller **and** callee (`aeneas.md`).
I/O *order* of those calls: `script.md`. Tokens for write-lock clients
(CapybaraKV RW-lock / VerusSync) are the data-race row — not `parking_lot`.
