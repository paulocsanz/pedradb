# RFC-0224 P2.3 recusa medida — ∀ do ConcurrentDb (A6)

**Date:** 2026-09-14
**HEAD:** `8ad49dc0`
**Axis:** A6 `concurrency_forall_theorems`
**Reaffirms:** `findings/2026-09-14-rfc0222-p24-concurrency-forall-recusa.md` (`e7864aaf`) with a new HEAD.

## Measured

```
python3 scripts/sel4_gap.py
  A6 ∀-concorrência: 0
```

The metric counts `\bconcurrent_db_\w*forall\w*\b`. Live = **0**. `glue.db_rs_extracted` = false; `concurrent.rs` is not extracted.

Writer-spine dual-unfold (RFC-0224 P0.1–P0.3) and group-commit member ∀ (`group_validate` / `occ_member_fate`) are reduction kernels, not a ∀ over ConcurrentDb lock order, flush-vs-rotate, and N-way group as one theorem. RFC-0220 P1.2/P2.1 still `todo`.

## What would close it

A theorem matching the metric that is ∀ over the rustc-linked ConcurrentDb write group, dual-unfold of the group-commit plan AND the `concurrent.rs` caller, zero `sorry`. Naming `concurrent_db_forall` without that statement is refused.

## Residual

A6 stays 0. Published, not skipped.
