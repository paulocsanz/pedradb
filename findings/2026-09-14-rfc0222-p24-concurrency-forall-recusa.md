# RFC-0222 P2.4 recusa medida — ∀-concorrência do ConcurrentDb

**Date:** 2026-09-14
**HEAD:** `e7864aaf`
**Axis:** A6 `concurrency_forall_theorems`

## Measured

```
python3 scripts/sel4_gap.py
  A6 ∀-concorrência: 0
```

The metric counts `\bconcurrent_db_\w*forall\w*\b`. Live = **0**.

`group_commit_kernel` has ∀ over OCC *members* (`group_validate` / `occ_member_fate` / `ComposeWriter` workerless gate). That is a reduction kernel, not a ∀ over `ConcurrentDb`'s lock order, flush-vs-rotate races, and N-way group as one theorem. RFC-0220 P1.2/P2.1 still `todo` for those rows.

## What would close it

A theorem matching the metric (or the metric widened in the same commit as the proof) that is ∀ over the rustc-linked `ConcurrentDb` write group, dual-unfold of the group-commit plan AND the concurrent.rs caller, zero `sorry`.

## Residual

A6 stays 0. The group-commit atoms remain islands-plus-workerless-compose, not ConcurrentDb-∀. Published, not skipped.
