# Concurrency proof map (data race / race condition / deadlock)

**Date:** 2026-09-07
**Question:** prove ConcurrentDb concurrency, data races, race conditions, deadlocks — nothing “out of scope”.
**Primary source:** Sharma, Dardinier, Parthasarathy, Pîrlea, Müller, Summers, *VerusBelt: A Formally Verified Semantic Model of Verus*, PLDI 2026. PDF: `2026-pldi-verusbelt.pdf` (Iris/Rocq soundness of Verus cells, invariants, lifetimes, concurrency, thread safety).

These are **four different theorems**. Mixing them is how “ConcurrentDb is out” leaked in.

## 1. Data race (UB)

Two threads access the same location, at least one write, no happens-before.

- **Rust AXM** (alias XOR mutability) is the first machine: safe Rust has no data races. Pedra `#![forbid(unsafe_code)]` on kernels; C ABI / posix unsafe is a named residual (`R-unsafe-*`), not a slogan.
- **VerusBelt (PLDI 2026):** first semantic soundness proof for a large Verus subset, including concurrency and thread safety, mechanized in Iris/Rocq. Cites CapybaraKV (verified concurrent KV in Verus) and NR / concurrent allocators.
- **CapybaraKV (OSDI’25 PoWER):** Verus KV; concurrency extension is reader-writer lock *or* sharding — the same two shapes Pedra already has (`ConcurrentDb` `RwLock` + OCC group). Not a proof of `parking_lot` itself.
- **seL4 multicore lock:** rely/guarantee in Isabelle for the seL4 ticket lock under weak memory (Springer 2024, *Practical Rely/Guarantee Verification of an Efficient Lock for seL4*). seL4 did **not** skip locks; they proved the lock algorithm.

Pedra today: type-system race freedom on safe paths; no Verus `AtomicInvariant` / storage-protocol proof of `RwLock` internals. Payable: treat `RwLock` as a mutex spec (exclusive writer / shared readers) and prove *clients* obey it — VerusSync / token protocol, same class as CapybaraKV’s RW-lock extension.

## 2. Race condition (logic)

Lost update, TOCTOU, “wrong interleaving still type-safe”.

- Pedra **OCC first-committer-wins** is already a Lean closed form: `occ_conflict` (`GroupCommit.lean`), called from `ConcurrentDb::lone_commit` / `validate_occ_batch`.
- **Group simultaneity:** `group_validate` — members of one group see the same `last_seq`; intra-group writes do not conflict. Lean `group_members_are_simultaneous`.
- **Serialized mutant** `occ_conflict_as_is_serialized` is the planted race: second same-group writer aborts where the group form commits.

This is the Pedra analogue of “atomicity violation” in the Petri-net Rust concurrency paper (arxiv 2212.02754): type system does not catch it; the kernel does.

## 3. Deadlock (wait-for cycle)

- **TransactionDB 2PL path (compat):** `wait_for_deadlock` in `locktab.rs` — follow `waiting → owned` until cycle or waiter. Lean extract + `wait_for_deadlock_is_loop` / as-is always false. Production `LockTable::lock` calls it when `detect`.
- **ConcurrentDb path is OCC + one write lock**, not 2PL. Deadlock shape here is **lock order**: write-group vs flush/GC. RFC-0042 `commit_inflight` is the named gate (flush skips WAL rotate instead of waiting on the write lock during off-lock fd). That gate is still mostly glue; it should become a kernel (`may_rotate_wal` already exists on the flush side) and a Lean dual-unfold like `on_barrier`.
- LockBud / Petri-net detectors are *bug finding*, not a theorem that Pedra’s wait-for is complete.

## 4. Scheduler / weak-memory ∀π

- `lock_interleavings_admitted = ok false` (Lean, 2026-09-07) is the **theorem that we do not claim ∀ OS schedules**, not a refuse of the topic.
- PCT d=2 is a **campaign** (`R-pct`, continuous). seL4’s multicore proof is rely/guarantee of *one lock*, not ∀π of the whole kernel under every scheduler.
- Next payable step toward ∀ of the *group protocol*: pull more of `finish_group_off_lock` / `lone_commit` into named kernels (collect OCC reads → `group_validate` → WAL → `may_publish_group` → apply). Glue shrinks; the remaining TCB is `parking_lot` + OS, same class as seL4’s assembly.

## Order of payment (nothing off the table)

1. Dual-unfold ConcurrentDb OCC path: `validate_occ_batch` facts → `group_validate` → `occ_conflict` (Lean already has the callees; caller is still in `concurrent.rs`). **Paid:** `occ_batch_plan_lagging_conflict`.
2. Dual-unfold publish: off-lock WAL result → `may_publish_group` (kernel exists; wire theorem from the ConcurrentDb claim methods). **Paid:** `may_publish_group_needs_wal_ok`.
3. Deadlock: keep locktab cycle as the 2PL theorem; name a ConcurrentDb lock-order kernel for flush vs group (commit_inflight / may_rotate). **Paid:** `wal_rotate_commit_inflight_keeps`. Write-lock OCC snap: `occ_snap_lock_order` (read-lock held vs writer exclusive) dual-unfolds `occ_snap_uses_published` — `ConcurrentDb::occ_snapshot` matches it.
4. Data race: Verus token protocol on `RwLock` clients (CapybaraKV RW-lock pattern / VerusSync), not a dump of `parking_lot`. Partial: `rwlock_client_may_mutate`. Reader token still unpaid.
5. Weak memory: only after (1)–(4), one lock algorithm like seL4’s ticket lock paper — not “the computer”.
6. N-way: **Paid** `group_validate_n3_one_lagging` unfolds `group_validate` ∧ `occ_conflict`; `occ_batch_plan_n3_one_lagging` unfolds the plan `validate_occ_batch` matches. Named test `occ_batch_plan_n3_one_lagging_is_not_ok`.

**Not claimed:** “sem data races no hardware”, “TSan is a proof”, “PCT d=2 = ∀π”.
