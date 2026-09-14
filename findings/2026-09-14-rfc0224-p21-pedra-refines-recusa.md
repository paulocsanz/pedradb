# RFC-0224 P2.1 recusa medida — teorema topo `pedra_refines` (A1)

**Date:** 2026-09-14
**HEAD:** `8ad49dc0`
**Axis:** A1 `top_theorems` (`scripts/sel4_gap.py` counts `\bpedra_refines\b`)
**Parent:** RFC-0222 P2.1 paid composition (`ComposeWriter`), not a unique refinement.

## Measured

```
python3 scripts/sel4_gap.py
  A1 refinamento topo (teorema único): 0
```

`rg pedra_refines formal/aeneas/lean` (minus `.lake`) = empty. `glue.db_rs_extracted` = false. The production write/lookup handlers live in the `db.rs` trampoline; Charon/Aeneas do not extract that file (canon RFC-0061 / RFC-0171).

A unique refinement "the rustc-linked Pedra handler is this spec" has no term: the handler is not in the extract.

## What would close it

A machine-checked `pedra_refines` whose statement is a refinement of the rustc-linked lookup/write path (not a kernel island, not a name-only lemma), zero `sorry`, that makes `top_theorems ≥ 1`. Dumping `db.rs` into the prover is still refused.

## Residual

A1 stays 0. DEFINING 35.15% after P1 does not include a top theorem. Published, not skipped. Naming a lemma `pedra_refines` without that statement is refused.
