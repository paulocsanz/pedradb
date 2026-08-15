# Aeneas extract + Lean theorems

**Date:** 2026-08-15

## Vote (`vote_kernel.rs`)

- Charon `0.1.232` + Aeneas `daa85d7` → `out/lean/VoteKernel.lean`.
- Production tweaks so the extract is not an axiom: `can_vote` and `grant_after_persist` use `match`.
- Lean 4.31.0 accepted (no `sorry` in `Vote.lean`):
  - `vote_decision_matches_spec`
  - `grant_after_persist_implies_ok` (wire grant ⇒ persist Ok)
  - `as_is_grants_where_fixed_denies` (mutant teeth)

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
- **Not claimed:** loop-level T1 (`insert` then `may_contain` over
  `Vec` + `IteratorRange`) or T2 encode/decode. Those stay Verus/Kani/tests.

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

> Lean accepted those named theorems of the Aeneas extracts of the production Rust files. Persist/disk remain axioms. F83 sibling is now Lean-∀ (`as_is_leaks_sibling`), not only Stateright. Bloom T4 (`always_true_never_rejects`) is Lean-∀ of the extract; insert-then-query is not.

Never: “Lean proved Raft / fold / the Bloom filter.”

## Re-check

```
./scripts/aeneas_vote.sh --required
./scripts/aeneas_isolated.sh --required
./scripts/aeneas_bloom.sh --required
./scripts/lean_vote.sh --required
./scripts/lean_bloom.sh --required
```
