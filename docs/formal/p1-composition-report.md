# P1 Composition Wave Report — RFC-0056 P1.1–P1.5

Status: **done** (2026-08-23). Composition of the crash dictionary
(`put` acked → WAL → recover → reopen → `get`), the second proof machine on
the remaining raft/WAL kernels, the TCP loop as a state machine, and the
vlog-GC / 2PC glue kernels. Evidence bar per slice: production calls the
pure kernel → Verus twin with named lemmas (`N verified, 0 errors`, zero
`sorry`) → AS-IS mutant with an in-repo counterexample → catalog fail-closed
(negative-tested) → RFC checkbox + Status row in the same change.

---

## 1. P1.1 — Dictionary theorem-link (put → WAL → recover → reopen → get)

- Kernel link twin: `crates/pedradb-core/verus/dictionary_link.rs` (315 LOC).
  Chained lemmas over the **existing production kernels**
  (`recover_collect_act`, `reopen_outcome`):
  - Axioms are **named hypotheses**, never hidden assumptions:
    - A1 `persist_axiom_holds(acked, wal)` — the persist axiom (put acked ⇒
      bytes in the WAL), prefix containment only (the WAL may carry an
      unacked suffix — equal lengths was a false first draft, caught while
      proving).
    - A2 `wal_all_records` — the WAL is well-formed records.
    - A3 `recovered_everything(recovered, wal)` — recovery returns the WAL.
    - A4 `tail_has_no_key(recovered, acked.len(), k)` — the unacked tail
      carries no newer version of the key under test (no shadow).
  - Top theorem: `lemma_put_acked_survives_crash` ⇒
    `reopen_outcome = ServeAll ∧ get_returns(k, v)` under A1–A4 + domination.
  - Teeth: `lemma_mutant_torn_tail_is_silent`,
    `lemma_mutant_reopen_swallows_damage`.
  - Verus: **8 verified, 0 errors**, zero `sorry`
    (`scripts/verus_dictionary_link.sh`).
- DST end-to-end test (exercises the real `Db`): `db.rs::
  crash_after_flush_and_tail_put_recovers_both_paths` — put "flushed" →
  flush → put "tail" → forget → reopen → both keys visible through the
  SST-inventory **and** WAL-replay paths.
- Catalog: `dictionary_link` pair (`data_fate`, handler `open_with_env`),
  lint negative-tested.

## 2. P1.2 — Second machine on the remaining kernels (Aeneas → Lean)

Extracts with drift-stamps (`sha256` of the kernel source; `--ci` fails on
drift) and Lean theorems over the **extracted** terms (not hand twins):

| Kernel | Stamp | Extract | Theorems (no `sorry`) |
|---|---|---|---|
| `wal/recover_kernel.rs` | `SOURCE.wal_recover` | `WalRecoverKernel.lean` | `WalRecover.lean` — 8 (CRC fail-closed, torn-prefix, orphan fragment + 3 AS-IS) |
| `wal/reopen_kernel.rs` | `SOURCE.reopen` | `ReopenKernel.lean` | `Reopen.lean` — 4 (clean serves all, fail-closed per damage, PIT, AS-IS swallows) |
| `apply_kernel.rs` (raft) | `SOURCE.apply` | `ApplyKernel.lean` | `Apply.lean` — 5 (closed form, done ≥ commit, missing stops, AS-IS applies hole) |

- `scripts/lean_wal_apply_reopen.sh` → `lake build` green.
- `check_extract` in `scripts/formal/pedra_formal.py` now verifies per
  kernel: artifact marker, stamp sha256, theorem file (named theorems, no
  `sorry`) and runs the lake build. `--ci`: **198 ok, 0 gap, 0 fail**.
- Negative test (triple mutation): bogus stamp sha / renamed theorem /
  injected `sorry` ⇒ exactly 3 distinct FAILs, restored green.
- Regression fixed while negative-testing: `--extract` regenerated
  `VoteKernel.lean` and destroyed the RFC-0053 P40 hand-patch
  (`Option::eq` as a match `def`, not an axiom). The patch is now
  re-applied automatically by `scripts/aeneas_vote.sh` after every
  regeneration, and the lake copy is kept in sync — re-verified by a full
  `--extract` run (`P40: Option::eq is a match def` green).

## 3. P1.3 — TCP loop as a state machine (whole-node model)

`crates/pedradb-raft/tests/tcp_node_model.rs` (376 LOC, Stateright, on the
**production** kernels `vote_decision`/`grant_after_persist`,
`ae_entry_action`, `may_commit_at`/`propose_ack_ok`, `apply_advance`).

- One action = ONE wire frame: *read frame → dispatch to the RPC kernel →
  persist → send reply*. The environment picks the next frame (that is the
  loop's real interleaving) and carries the persist outcome.
- Reduction claim (recorded, not hand-waved): the loop adds no state beyond
  (frame, node state); the model state is exactly the node's durable state
  plus send-time witness flags (an RPC log was the first draft — it makes
  every state unique and the BFS unbounded; witness flags are the finite
  encoding of the same observable).
- Invariants: `Inv-vote-once`, `Inv-F16-no-rewrite-committed`,
  `Inv-F11-no-dirty-ack` (witnessed **at send time** — a final-state check
  is maskable by later commit advance), `Inv-send-after-persist` (a grant
  reply implies the vote persisted Ok), `Inv-apply-contiguous`,
  `Inv-applied-le-commit`, + non-vacuity properties.
- Loop-level AS-IS mutants, each must counterexample:
  - `SendGrantBeforePersist` — replies the grant computed at read time,
    persist outcome ignored ⇒ breaks `Inv-send-after-persist`.
  - `DirtyAckLoop` — replies Ok without the commit-cover gate ⇒ breaks
    `Inv-F11`.
  - `SkipAeGuard` — installs AppendEntries without the AE kernel ⇒ breaks
    `Inv-F16`.
- Tests: **4/4 green** (fixed asserts all properties; three mutant tests
  assert the counterexample exists). Registered in the catalog `models`
  list → runs in `--ci` (`check_models`).

## 4. P1.4 — vlog GC + 2PC glue kernels

### vlog GC (`crates/pedradb-core/src/vlog_gc_kernel.rs`, 246 LOC)

- `vlog_recover_action(blob_active, wants_large, primary_exists, use_new,
  new_exists)` — the MANIFEST-swing crash decision, extracted from
  `open_with_env` + `ValueLog::resolve_path`/`open_with_flag`:
  committed swing + staged `.new` ⇒ `OpenNew`; orphan `.new` (crash before
  commit) ⇒ primary, never `.new`; promote-rename-done ⇒ primary
  (reconcile); both files gone under a committed swing ⇒ `RefuseOpen`
  (F51 — inventing an empty primary makes every large value vanish); fresh
  DB ⇒ `CreateEmptyPrimary`; nothing configured ⇒ `NoVlog`.
- `blob_gc_action(is_active, bytes)` — the sealed-blob rewrite guard
  (never the active append generation, never an empty file); θ dead-ratio
  stays a policy float applied by the caller **after** the guard.
- Production: `open_with_env` matches the kernel action (flag-resolved arms
  delegate to `open_with_flag`, which keeps the same F51/empty-create rules
  as defense-in-depth); `compact_blob_auto` picks via the kernel guard.
- Mutants: `vlog_recover_action_as_is_ignore_swing` (serves the stale
  pre-GC primary after a crash mid-GC — the silent resurrect/vanish bug),
  `blob_gc_action_as_is_rewrite_active` (rewrites the active generation —
  concurrent appends vanish). Finite-domain theorem tests over 2^5 and the
  (active × bytes) grid assert the divergences.
- Twin `crates/pedradb-core/verus/vlog_gc_decision.rs`: **14 verified,
  0 errors**, zero `sorry` (`scripts/verus_vlog_gc.sh`); named lemmas
  `lemma_swing_opens_staged_new`, `lemma_orphan_new_never_opened`,
  `lemma_promote_done_reconciles_primary`, `lemma_f51_refuses_both_missing`,
  `lemma_fresh_db_creates_empty_primary`, `lemma_blob_mode_wins`,
  `lemma_active_generation_never_rewritten`,
  `lemma_empty_file_never_rewritten` + mutant teeth
  `lemma_mutant_serves_stale_primary_after_swing`,
  `lemma_mutant_invents_empty_primary_on_f51`,
  `lemma_mutant_rewrites_active_generation`.
- Catalog: `vlog_recover` (handler `open_with_env`) + `blob_gc_pick`
  (handler `compact_blob_auto`).

### 2PC glue (`crates/pedradb-store/src/tx_glue_kernel.rs`, 93 LOC)

- `tx_range_action(range_committed, tx_failed)` — per-range cleanup when a
  multi-range `TxnCommit` fails: already-majority-committed ⇒
  `MajorityRevert` (majority `TxnRevert` on the same raft log, F47 — a
  local-only cleanup leaves the user-key apply visible and the TX stops
  being all-or-nothing); never-committed ⇒ `LocalRevert` (F34); success ⇒
  `KeepCommitted`.
- Production: `tx_finish`'s cleanup loop matches the kernel action per
  range.
- Mutant `tx_range_action_as_is_local_only` — finite-domain theorem asserts
  divergence exactly on (committed, failed).
- Twin `crates/pedradb-store/verus/tx_glue.rs`: **7 verified, 0 errors**,
  zero `sorry` (`scripts/verus_tx_glue.sh`); named lemmas
  `lemma_committed_range_gets_majority_revert`,
  `lemma_uncommitted_range_gets_local_revert`,
  `lemma_success_keeps_every_range` + teeth
  `lemma_mutant_leaves_majority_apply_visible`.
- Catalog: `tx_glue` (handler `tx_finish`).

Negative lint test (all five P1.4 handler names renamed at once):
**5 FAILs** (`vlog_recover`, `blob_gc_pick`, `tx_glue` + the two pre-existing
pairs that share `open_with_env` — `reopen_outcome`, `dictionary_link`),
restored to green. (First two attempts of this negative test were
**invalid**: macOS `sed` silently ignores `\b`, so the rename never
happened — recorded here because a negative test that cannot fail is worse
than none.)

## 5. LOC summary

| Layer | P1.1+P1.4 kernels | Twins | Model |
|---|---|---|---|
| LOC | 339 (dictionary link has no new kernel — it links the existing ones) | 688 (dictionary_link 315, vlog_gc 273, tx_glue 100) | 376 (`tcp_node_model.rs`) |

Verus totals this wave: dictionary_link 8 + vlog_gc 14 + tx_glue 7 =
**29 verified, 0 errors**, zero `sorry` (plus 17 Lean theorems over the
extracts).

## 6. TCB delta

- The vlog swing decision (F51) and the 2PC per-range cleanup (F47/F34)
  leave the "inline glue" set and become kernel+twin pairs — the same
  shape as the P0 engine kernels.
- The persist axiom (A1) is now a **named, checkable hypothesis** in the
  dictionary link — the acked-put path is the composition of kernels, and
  the only remaining assumption is exactly the durability contract itself.
- No new axioms: the `Option::eq` axiom in the vote extract is a def again,
  permanently (script-applied).

## 7. What this wave does NOT claim

- Liveness/eventual election: P2.2 (needs the eventual-synchrony axiom).
- The `ConcurrentDb` interleaving: P2.1, gated on RFC-0051 PCT.
- f64 θ-threshold arithmetic is policy, deliberately **outside** the kernel
  (the guard covers only the structural safety conditions).
