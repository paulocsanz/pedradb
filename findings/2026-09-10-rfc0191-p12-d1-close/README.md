# RFC-0191 P1.2 — D1-script `model→close`

leftover_next: product remaining RFC-0191 P1.2.

## What this is

`d1_wal_commit_plan` : ∀ (need_sync sync_fail : Bool) over the extracted
`wal_commit_plan` (the plan `commit_ops_with` matches). Covers the whole
Bool×Bool space:

- need_sync ∧ ¬sync_fail ⇒ AppendSyncApplyOk
- need_sync ∧ sync_fail ⇒ AppendSyncFence (never Apply/Ok)
- ¬need_sync ⇒ AppendApplyOk

Concrete `wal_commit_plan_need_sync_ok` (`true false`) does **not** pay
this. As-is dente already in `wal_commit_plan_as_is_dente`.

Product TSV: D1 `model→close`, catalog:wal_commit_plan, floor_promoted 1→2.

## Why this is not a second depth-floor close

`wal_commit_plan` is already an Aeneas extract (`twin_kind=close` with
extract ⇒ proof_depth=extract). Registering it in `close_proofs.tsv`
makes `pedra_formal.py` count it as close **instead of** extract:
extract 276→275, below `floor_extract 276`. Floors only move up.

Same unique-pair lesson as P1.1 (second atom row on `catalog:visible_at`
did not raise live atom). Depth-floor stays: registered close=1
(`merge_sift`), atom=1 (`visible_at`). Product close ≠ catalog close.

## "ue regrediu?"

No. After P1.1, `floor_atom` stayed 1 because live atom is unique catalog
pair. Product R1 advanced (`r1_get_atom` both arms). Depth-floor GREEN
extract=276 close=1/1 atom=1/1.
