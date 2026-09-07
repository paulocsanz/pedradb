# RFC-0172 — translation validation, one target, one file

**Date:** 2026-09-06
**Residual:** `R-rustc` stays `never`. This is not a rustc proof.

Pinned host: `aarch64-apple-darwin`, rustc `1.97.1`
(`8bab26f4f68e0e26f0bb7960be334d5b520ea452`), LLVM `22.1.6`, `opt-level=0`.

Dump: `write_admission.ll` (unoptimized LLVM IR of production
`write_admission_kernel.rs`). Object is rebuilt by
`scripts/tv_write_admission.sh`; `SOURCE.tv` binds kernel sha256 + IR sha256.

TV (P2.1): restricted interpreter of `write_admission_idle` over all 8
boolean inputs vs `!mem && !pressure && !stall`. Gate fails if rustc's
translation of that fn disagrees with the spec.
