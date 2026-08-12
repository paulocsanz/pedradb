# Montanha P0–P1: invariants ↔ tests (RFC-0013)

**Status:** living map for P0 + P1.1 multi-key  
**Updated:** 2026-08-12  
**Contract:** [RFC-0013](rfc/0013-montanhadb-product.md) §5 invariants, §9 catalogue, P1.1  
**Code:** `crates/pedradb-store`, apply helpers in `crates/pedradb-dcs`

---

## How to run gates

```bash
# Full store suite (primary L1)
cargo test -p pedradb-store --lib

# DCS apply / local DCS (when apply path changes)
cargo test -p pedradb-dcs --lib

# Clippy (store path)
cargo clippy -p pedradb-store --all-targets -- -D warnings
```

Optional filters (logical suites):

```bash
cargo test -p pedradb-store --lib multi_range
cargo test -p pedradb-store --lib majority_durable
cargo test -p pedradb-store --lib put_fails_without_majority
cargo test -p pedradb-store --lib put_batch
cargo test -p pedradb-store --lib minority_only
cargo test -p pedradb-store --lib strong_read
cargo test -p pedradb-store --lib failover
cargo test -p pedradb-store --lib dcs_on_store
cargo test -p pedradb-store --lib dcs_create_fails
cargo test -p pedradb-store --lib dcs_create_not_committed_heal
cargo test -p pedradb-store --lib dcs_apply_cas_failed
```

---

## Consistency names (I-RD)

| Policy | API | Linearizable? | Fencing? |
|--------|-----|---------------|----------|
| **LocalApplied** | `get` / `get_on` / `get_with_policy(..., LocalApplied)` | **No** | **No** — UI / lag-tolerant only |
| **Strong** | `get_strong` / `get_with_policy(..., Strong)` | Leader/revalidated class (P0 MVP) | Use with unique safe leader only |

Dual `Role::Leader` claims ⇒ `range_leader` is `None` and Strong fails on **all** claimants.

---

## Invariant → test map

### Multi-range (I-MR-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-MR-1 concurrent range puts | `multi_range_puts_different_leaders` | pedradb-store |
| I-MR-2 locate total | `multi_range_puts_different_leaders`, `range_contains` | pedradb-store |

RFC §9 id: `T-MR-multi-range-puts`.

### Majority durability (I-MAJ-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-MAJ-1 Ok ⇒ majority applied | `majority_durable_put_on_three_peers` | pedradb-store |
| I-MAJ-2 minority cannot Ok | `put_fails_without_majority_under_partition`, `dcs_create_fails_without_majority_under_partition` | pedradb-store |
| I-MAJ-2 (local-only hook) | `minority_only_append_does_not_commit` | pedradb-store |
| I-MAJ-3 no silent orphan commit | `dcs_create_not_committed_heal_retry_put_ok` (in-process heal+tick); `dcs_create_not_committed_survives_reopen_without_installing_lock` (durable discard + reopen) | pedradb-store |
| I-MAJ-4 apply pipeline not stuck | `dcs_apply_cas_failed_does_not_stick_pipeline`, `dcs_create_not_committed_heal_retry_put_ok` (put x/y after) | pedradb-store |

RFC §9: `T-MAJ-*`.

### Reads (I-RD-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-RD-1 named policies | `strong_read_refuses_deposed_and_dual_leader` + this doc | pedradb-store |
| I-RD-2 dual-leader fail closed | `strong_read_refuses_deposed_and_dual_leader` | pedradb-store |
| I-RD-3 LocalApplied non-fencing | this doc + `montanha-vs-foundationdb.md` | docs |

RFC §9: `T-RD-*`.

### Range HA (I-HA-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-HA-1..3 | `range_failover_after_leader_loss` | pedradb-store |

RFC §9: `T-HA-leader-loss`.

### DCS-on-store (I-DCS-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-DCS-1..4 create/cas/replicate | `dcs_on_store_create_replicated` | pedradb-store |
| I-DCS-4 partition | `dcs_create_fails_without_majority_under_partition` | pedradb-store |
| I-DCS-5 heal/retry no brick | `dcs_create_not_committed_heal_retry_put_ok`; `dcs_create_not_committed_survives_reopen_without_installing_lock` | pedradb-store |
| Apply idempotent Create | `command::tests::encode_round_trip_and_apply_cas`, `dcs_apply_cas_failed_does_not_stick_pipeline` | pedradb-dcs / store |

RFC §9: `T-DCS-*`, `T-DCS-APPLY-*`.

### In-range multi-key atomic (P1.1 / I-MK-*)

| Invariant | Test function | Crate |
|-----------|---------------|-------|
| I-MK-1 Ok batch ⇒ all keys majority-applied | `put_batch_same_range_majority_atomic` | pedradb-store |
| I-MK-2 index-style row+secondary in one batch | `put_batch_row_and_secondary_index_style` | pedradb-store |
| I-MK-3 minority ⇒ NotCommitted, no partial majority | `put_batch_fails_without_majority_no_partial` | pedradb-store |
| I-MK-4 durable batch across reopen | `put_batch_survives_reopen` | pedradb-store |
| I-MK-5 multi-range single-key still works after batch API | `put_batch_cross_range_hard_fails` (tail puts), `multi_range_puts_different_leaders` | pedradb-store |

API: [`StoreCluster::put_batch`](../../crates/pedradb-store/src/lib.rs) — one raft log entry `RangeEntry::Batch`, apply via PedraDB `apply_batch`.

### Cross-range policy (P1 honesty / I-XR-*)

| Policy | Behavior | Test |
|--------|----------|------|
| **Hard-fail (shipped)** | Keys locating to ≥2 ranges → `StoreError::CrossRange`; **zero** partial apply | `put_batch_cross_range_hard_fails` |
| Global FDB-style cross-range TX | **Not shipped** — layers must co-locate keys or split ops | — |

This is **TiKV-store-class** co-location for multi-key atomicity, not FDB global TX. Documented so layers do not overclaim.

### PedraDB boundary (I-PEDRA-*)

| Invariant | Evidence |
|-----------|----------|
| I-PEDRA-1 one dir one process | Store opens `store-node-{id}` per peer; exclusive PedraDB open |
| I-PEDRA-2 no Raft in core | Raft only in `pedradb-store` / `pedradb-raft` |

### Multi-process store (P1.4)

| Status | Note |
|--------|------|
| **Deferred** | Network multi-Raft store binary not required for P1 multi-key; reopen when `pedradb-store` has multi-process elect+put harness (or reuse pattern from `pedra-raft-node` multi_process tests for store). In-process gates I-MK-* remain the bar. |

### Extra durability (supporting P0)

| Test | Role |
|------|------|
| `raft_meta_survives_reopen` | Raft hard/log meta + data across reopen |
| `raft_log_compacts_after_all_applied` | Log prefix compact; data retained |
| `put_rejects_raft_meta_prefix` | Client cannot clobber internal raft meta keys |
| `open_rejects_too_many_ranges` | Safe keyspace split bounds |
| `leader_noop_commits_prev_term_after_reelect` | HA after step-down / re-elect |

---

## Adversarial checklist (RFC-0013 §8)

| Attack | Expected | Covered by |
|--------|----------|------------|
| Ok without majority | Err `NotCommitted` / NotLeader | `put_fails_without_majority_under_partition` |
| Strong on dual leader | Err `StaleLeader` both | `strong_read_refuses_deposed_and_dual_leader` |
| NotCommitted then heal installs lock | Key still absent after tick | `dcs_create_not_committed_heal_retry_put_ok` |
| Retry after NotCommitted bricks puts | Puts still Ok after heal+retry | same |
| Docs claim LocalApplied linearizable | Forbidden | this doc |
| Partial multi-key under partition | NotCommitted; no key on majority | `put_batch_fails_without_majority_no_partial` |
| Cross-range silent partial apply | `CrossRange`; zero apply | `put_batch_cross_range_hard_fails` |
| Multi-range collapsed to single writer | Still multi-range puts Ok | `multi_range_puts_different_leaders` + cross-range test tail |

---

## Bootstrap vs product

| Path | Role |
|------|------|
| `pedradb-store` | **Montanha-Store L1** (P0 substrate) |
| `pedradb-raft` + `pedra-raft-node` | **Bootstrap** single-domain network demo — not product identity |

New coordination features prefer the store (RFC-0013 §4.4).
