# RFC-0224 P2.2 recusa medida — confinamento (A3)

**Date:** 2026-09-14
**HEAD:** `8ad49dc0`
**Axis:** A3 `confinement_theorems`
**Reaffirms:** `findings/2026-09-14-rfc0222-p23-confinement-recusa.md` (`e7864aaf`) with a new HEAD.

## Measured

```
python3 scripts/sel4_gap.py
  A3 confinamento (2º teorema): 0
```

`rg '\bconfinement\b' formal/aeneas/lean` (minus `.lake`) = empty. Lookup/scan kernels (`lookup_kernel`, `scan_readahead_kernel`) have fate-iff atoms, not a confinement predicate ("this key cannot be observed by that client / CF / tenant").

The 0222 refusal still holds: fail-closed CRC and CF encode atoms are not an information-flow theorem.

## What would close it

A theorem whose statement is confinement over the production lookup/scan path rustc links, machine-checked, zero `sorry`, that makes `confinement_theorems ≥ 1`. Naming a lemma `confinement` without that statement is refused.

## Residual

A3 stays 0. DEFINING 35.15% after P1 is recovery+surface, not confinement. Published, not skipped.
