# RFC-0222 P2.5 recusa medida — translation validation rustc/LLVM

**Date:** 2026-09-14
**HEAD:** `e7864aaf`
**Axis:** binary / `R-rustc` never_floor
**Parent:** RFC-0172 (status done as *one-target spec*; residual `R-rustc` remains `never`)

## Measured

RFC-0172 closed the *spec* of one target (`write_admission_kernel.rs`, pinned rustc/LLVM, MIR/bitcode vs extract). It did **not** land a machine-checked correspondence artifact. `never_floor` still lists `R-rustc`. No `pedra_refines` theorem. No object-file TV harness in CI.

## What would close it

A pinned (host, rustc, crate, file) TV run that checks the object rustc emits for `write_admission_kernel.rs` against the Lean/Verus term, fail-closed, registered. That still does not prove rustc; it proves one object. `R-rustc` stays `never` either way.

## Residual

P2.5 is research-scale (seL4 paid years after C). Published refusal: no TV artifact in-tree. Successor RFC-0224 names it as a P2 terminal, not a silent skip.
