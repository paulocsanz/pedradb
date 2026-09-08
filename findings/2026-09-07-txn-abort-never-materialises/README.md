# Abort never materialises user keys (F47)

**Date:** 2026-09-07
**Primary source:** Khan, *Verified Detection and Prevention of Concurrency
Anomalies in Multi-Agent Large Language Model Systems*, arXiv:2606.17182
(15 Jun 2026). Verus-checked SSI runtime: `ssi_commit_abort_step` vs
`ssi_commit_success_step` — abort on validation does not install the
write-set. Excerpt: `arxiv-2606.17182-excerpt.md`.

This is not Pedra's raft TX apply. It is the class of guarantee: an abort
status must not take the materialise arm.

## Used this turn

Catalog pair `txn` (`data_fate`, entry `txn_commit_action`): abort ⇒
`Revert`, never `Materialise`. AS-IS always materialises. Production
`txn_kernel.rs` is now the Verus term (`single_artifact`). Lemma
`lemma_as_is_materialises_abort` is the second possibility. Other pairs on
the same file keep the twin until their turns.

## Not claimed

Khan's LLM-agent lattice. Dump of `apply_txn_commit`. “somos seL4”.
