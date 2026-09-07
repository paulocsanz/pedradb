# Verified kernel inside an unverified crate (CapybaraKV / PoWER, OSDI'25)

**Date:** 2026-09-07
**Primary source:** LeBlanc, Lorch, Hawblitzel, Huang, Tao, Zeldovich, Chidambaram, *PoWER Never Corrupts: Tool-Agnostic Verification of Crash Consistency and Corruption Detection*, OSDI 2025. PDF: `osdi25-leblanc.pdf`.

## What the paper actually does

CapybaraKV is a persistent-memory KV store written in Verus (~25 kLOC, 15 k proof) and **integrated with an unverified Rust codebase**. The slides (OSDI'25) name that integration as a design constraint: the verified core is what rustc links; the surrounding crate is not dumped into the prover. Crash-atomic operations; verifies in <1 min.

That is RFC-0171 / RFC-0174 seL4 price on a storage kernel: the production `.rs` is the Verus term, glue stays trampoline.

## Used this turn

Catalog pair `tx_glue` (`data_fate`, was twin ≠ kernel): `crates/pedradb-store/src/tx_glue_kernel.rs` is now `single_artifact`. `tx_range_action` is the 2PC per-range cleanup decision `StoreCluster::tx_finish` (unverified `lib.rs`) already calls. Verus lemmas F47/F34 (`lemma_committed_range_gets_majority_revert` / `lemma_mutant_leaves_majority_apply_visible`) live in that same file.

## Not claimed

Dump of `store/src/lib.rs` or `db.rs`. “somos seL4”. CapybaraKV’s PM/PoWER checksum primitive is a different residual (`R-crc` stays `never`).
