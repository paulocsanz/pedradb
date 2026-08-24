# Aeneas extract + Lean theorems

**Date:** 2026-08-15

## Vote (`vote_kernel.rs`)

- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/VoteKernel.lean`.
- Production tweaks so the extract is not an axiom: `can_vote` and `grant_after_persist` use `match`.
- Lean 4.31.0 accepted (no `sorry` in `Vote.lean`):
  - `vote_decision_matches_spec`
  - **`vote_decision_iff`** (RFC-0053 P40: iff on the extracted term)
  - `grant_after_persist_implies_ok` (wire grant ⇒ persist Ok)
  - `as_is_grants_where_fixed_denies` (mutant teeth)
- **P40 Option::eq:** `VoteKernel.lean` models `Option::eq` as `match` (`def`, not `axiom`). `pedra_formal.py` fails if an Aeneas re-extract restores the axiom.

## Bloom (`bloom.rs`, RFC-0030)

- `[lib] path` = production `crates/pedradb-core/src/bloom.rs`.
- Production tweaks so the extract typechecks / is not an axiom on the T1/T4
  path (same method as Isolated/`starts_with`):
  - `bit_index`: bound + cast, not `try_from`/`unwrap_or`.
  - `is_active`: `len() != 0`, not `!is_empty()`.
  - `with_capacity`: explicit `saturating_product` / `at_least_64` /
    `cap_u32` / `k_from_bits_per_key` / `nbytes_for_nbits` — no
    `Ord.max`/`min`/`clamp`/`div_ceil` (those failed Lean typecheck or
    extracted as axioms). Same integer form as before
    (`with_capacity_matches_legacy_formula`).
- Remaining axioms live on decode/error paths only: `U64.div_ceil`
  (`bloom_header_ok`), `Result.unwrap_or` (`encode` length), `fmt.format`
  / `String.from` (decode errors), `must_use`, `RangeInclusive.contains`.
- Lean 4.31.0 accepted (no `sorry` in `Bloom.lean`):
  - `may_contain_nbits_zero` / `may_contain_k_zero` / `always_true_never_rejects` (T4)
  - `set_bit_test_bit_same` (T1 core: extracted `set_bit` then `test_bit`
    on the same index, given `i/8` in bounds)
  - `probe_bit_lt` — extracted `probe_bit` is `< nbits` when `nbits ≠ 0`
  - `bit_index_of_le_u32max` — `bit ≤ u32::MAX` ⇒ `bit_index` is the cast
- Production `insert` / `may_contain` are Isolated-style `while i < k`
  over a shared `probe_bit` (range-for extracted to `IteratorRange` and
  blocked the loop proof).
- **Not claimed:** `insert_loop` then `may_contain_loop` composition
  (Vec deref_mut + k-step invariant). T2 encode/decode. Those stay
  Verus/Kani/tests.

## AE (`ae_kernel.rs`, RFC-0053 P2.1)

- Charon + Aeneas → `out/lean/AeKernel.lean` (`SOURCE.ae` sha256).
- Lean 4.31.0 accepted (no `sorry` in `Ae.lean`):
  - `ae_keep_if_same_term` (∀)
  - `ae_refuse_conflict_at_commit` / `ae_truncate_conflict_after_commit`
  - `as_is_rewrites_committed` (F16 teeth)

## Commit (`commit_kernel.rs`, RFC-0053 P2.1)

- Charon + Aeneas → `out/lean/CommitKernel.lean` (`SOURCE.commit` sha256).
- Lean 4.31.0 accepted (no `sorry` in `Commit.lean`):
  - `may_commit_at_iff` (∀)
  - `recover_commit_caps_examples` (F10 min)
  - `as_is_commits_prev_term` / `as_is_recover_promotes_suffix`

## Isolated (F83, was a Verus cartoon)

- `[lib] path` = production `isolated_kernel.rs`.
- `starts_with` extracted as an axiom; kernel rewritten to a byte loop (same predicate). Re-extract is **axiom-free**.
- Lean 4.31.0 accepted (no `sorry` in `Isolated.lean`):
  - `isolated_id_matches_loop_spec` / `_spec` — ∀ prefix + (`=` ∨ next `'/'`)
  - `isolated_id_matches_as_is_loop_spec` / `_spec` — ∀ prefix only
  - `as_is_leaks_sibling` — `/vm/vm-a` vs `/vm/vm-ab` (F83 teeth)
  - `isolated_id_matches_too_short`, child-byte atoms
- Method that closed: `loop.spec_decr_nat` + `step` on `index_usize_spec`;
  lengths only as `x.val.length : Nat`; no `simp [loop]`.

## What we may say

> Lean accepted those named theorems of the Aeneas extracts of the production Rust files. Persist/disk remain axioms. F83 sibling is now Lean-∀ (`as_is_leaks_sibling`), not only Stateright. Bloom T4 and the T1 bit-core (`set_bit_test_bit_same`) are Lean-∀ of the extract; insert-loop then query-loop is not.

Never: “Lean proved Raft / fold / the Bloom filter.”

## Re-check

```
./scripts/aeneas_vote.sh --required
./scripts/aeneas_ae.sh --required
./scripts/aeneas_commit.sh --required
./scripts/aeneas_isolated.sh --required
./scripts/aeneas_bloom.sh --required
./scripts/lean_vote.sh --required
./scripts/lean_ae_commit.sh --required
./scripts/lean_bloom.sh --required
```
