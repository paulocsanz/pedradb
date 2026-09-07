# Production Rust is the proof term (Atmosphere / Aeneas pipeline)

**Date:** 2026-09-07
**Primary source (reachable PDF):** Klaus, Conejero, Tolmach, *A Rust-to-Lean Verification Pipeline with AI Provers: An Experience Report*, arXiv:2605.30106 (May 2026). PDF: `2605.30106.pdf`.
**Adjacent (ACM 403 this turn):** Narayanan et al., *Atmosphere: Practical Verified Kernels with Rust and Verus*, SOSP 2025, doi:10.1145/3731569.3764821 — Verus on the executable microkernel, not a twin-cópia. Not cached here.

## What 2605.30106 actually does

Stage 1 runs Charon on a **Cargo crate of production Rust**, then Aeneas to Lean 4 (`Result` monad: ok / fail / div). Goal: “verify production Rust as written.” Where the toolchain lacks a feature they re-model a fragment and pin it with compile-time assertions; the rest goes through unmodified.

Atmosphere (SOSP’25 abstract, not the PDF this turn): 6 kLOC executable Rust proved in Verus as a refinement of a high-level spec — the binary’s kernel is the term.

## Used this turn

Catalog pair `apply_step` (`data_fate`, was twin ≠ kernel): `crates/pedradb-raft/src/apply_kernel.rs` is now `single_artifact`. `apply_advance` is the F10 apply-loop decision `RaftNode::apply_committed` already calls. Clone with store `apply_kernel.rs` stays token-identical on the rustc body (last-wins). Aeneas SOURCE.apply re-stamped on the same production file.

## Not claimed

Dump of `raft/src/lib.rs` apply loop glue. “somos seL4”. Atmosphere’s isolation theorems are a different kernel.
