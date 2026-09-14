# RFC-0222 P2.3 recusa medida — confinamento (2º teorema seL4)

**Date:** 2026-09-14
**HEAD:** `e7864aaf`
**Axis:** A3 `confinement_theorems` (`scripts/sel4_gap.py`)

## Measured

```
python3 scripts/sel4_gap.py
  A3 confinamento (2º teorema): 0
```

`sel4_gap.py` counts `\bconfinement\b` in `formal/aeneas/lean/**/*.lean` (minus `.lake`). Live = **0**.

seL4's second theorem is integrity/confinement of the kernel against user processes. Pedra has no corresponding ∀ over a rustc-linked confinement predicate (tenant/namespace/CF isolation as "this key cannot be observed by that client").

## What would close it

A theorem whose statement is a confinement property over the production lookup/scan path (not a toy model), machine-checked, zero `sorry`, registered, that makes `confinement_theorems ≥ 1`. Naming a lemma `confinement` without that statement is refused.

## Residual

A3 stays 0. DEFINING block 24.55% after P2.2 does not include a confinement theorem. Published, not skipped.
