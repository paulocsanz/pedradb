# Verus cartoon is not the term

Date: 2026-09-09

RFC-0171 / 0174: the data-fate `if` the binary runs lives in the file rustc
links. The term is that fn with the types the handler passes (`key::ValueType`,
`&[u8]`, `Bound`).

## Lies this campaign treated as paid (invalid)

1. `skip_verus_last_wins` — any `cfg(verus_keep_ghost)` in the file = last-wins.
   Dual-view `merge.rs` (`verus! { enum ValueType; range_tombstone_covers(u64,…) }`
   while rustc links `key::ValueType` + `&[u8]` + `Bound`) is **not** paid.
   Proving the cartoon does not prove the handler.
2. Minting 28 u64 Verus twins as rank-7 lands.
3. `leftover_next FACTORY_BAN` as a halt when ranks 4–10 are empty. Empty
   4–10 is cartoon remaining then trampoline remaining.
4. "I'll say the cartoon is a lie, then finish Bound-helper" — Bound-helper
   (`d1f4978f`) does not pay the cartoon debt.
5. Search `twin==kernel` + `single_artifact` = paid **before** checking the
   file for a `verus!` stand-in. That hid `visible_at` as catalog_only_skip.
6. `_body!` over different types (u64 vs `&[u8]`, `Seq<u8>` vs `&[u8]`, toy
   `enum ValueType` vs `key::ValueType`) is the same lie with a macro fig leaf.
7. Skipping cartoon remaining as "not a land" so the grind never pays it.

## What counts (the term)

- Handler calls the rustc fn with the types it already passes.
- Aeneas extract of **that** body (`scripts/aeneas_*.sh --required`) then Lean
  `unfold` of caller and callee.
- Verus last-wins only when it type-checks **those types** (`macro_rules!`
  over types both compilers share: bool, u64 counters, enums defined in the
  same file — `write_admission_kernel.rs` `idle_body!`).

## Payment for an existing cartoon

Delete the `verus!` stand-in and the `cfg(verus_keep_ghost)` split. rustc body
stays. Aeneas of that body is the proof. Do not mint a replacement twin. Do
not `_body!` over different types.

## What does not count (not a land)

- `verus!` toy enum / `u64` / `Seq<u8>` stand-in while rustc has bytes.
- `#[cfg(not(verus_keep_ghost))]` wrap billed as last-wins.
- leftover `is_empty` / `||` / `==` identity kernel.
- `FACTORY_BAN` halt.
- Bound-helper billed as cartoon payment.

Board: `skip_verus_same_tokens` (`_body!` same types) vs `cartoon_twin`
(unpaid — leftover_next names the kernel file; delete the stand-in).

This fire: deleted the `merge.rs` stand-in. rustc `visible_at` /
`range_tombstone_covers` (`key::ValueType` / `&[u8]`) stay. Term is Aeneas
`formal/aeneas/lean/Merge.lean`. `scripts/verus_visible_at.sh` fail-closes
if the cartoon returns.
