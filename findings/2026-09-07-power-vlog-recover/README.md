# PoWER: recoverability is a write precondition (vlog swing)

**Date:** 2026-09-07
**Primary source:** LeBlanc, Lorch, Hawblitzel, Huang, Tao, Zeldovich, Chidambaram, *PoWER Never Corrupts: Tool-Agnostic Verification of Crash Consistency and Corruption Detection*, OSDI 2025. PDF: `power.pdf` (full paper); artifact appendix: `osdi25-leblanc.pdf`.

## What the paper actually says

PoWER (Preconditions on Writes Enforcing Recoverability) encodes crash consistency in the **preconditions of storage API methods**, using only Hoare logic, ghosts, and quantifiers (no extra crash logic). A durable update is only admitted if every crash state it can produce is recoverable. They verify CapybaraKV in Verus this way. This is not Pedra's vlog; it is the class of guarantee for a MANIFEST swing.

## Used this turn

Catalog pair `vlog_recover` (`data_fate`, entry `vlog_recover_action`): committed swing + staged `.new` ⇒ `OpenNew`, never the stale primary. AS-IS ignores the swing flag. Production `vlog_gc_kernel.rs` is now the Verus term (`single_artifact`). Lemma `lemma_mutant_serves_stale_primary_after_swing` is the second possibility. Pair `blob_gc_pick` keeps the twin until its turn.

## Not claimed

PoWER of all Pedra Env writes. Dump of `db.rs` vlog open glue. CapybaraKV's PM model. “somos seL4”.
