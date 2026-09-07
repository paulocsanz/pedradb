# CrashMonkey found a crash-consistency bug in verified FSCQ

**Date:** 2026-09-07
**Primary sources:**
- Mohan, Martinez, Ponnapalli, Raju, Chidambaram, *Finding Crash-Consistency Bugs with Bounded Black-Box Crash Testing*, FAST'19 / arXiv:1810.02904. PDF: `1810.02904.pdf`.
- *Crash-Consistent Checkpointing for AI Training on macOS/APFS*, arXiv:2511.18323v1 (23 Nov 2025). PDF: `2511.18323.pdf` (cites CrashMonkey; 430 crash-injection trials, every unsafe checkpoint unusable).

## What the sources actually say

CrashMonkey + ACE implement bounded black-box crash testing (B3). They reproduced 24/26 reported crash-consistency bugs and found 10 new ones in mature Linux file systems. They also found a crash-consistency bug in **FSCQ, a verified file system** — a proof of a model did not prevent a post-crash image from serving a wrong state.

2511.18323: every crash injected into unsafe-mode checkpoints (430 trials) left the file corrupted or unusable. POSIX `rename` is not always atomic w.r.t. crashes.

## Used this turn

Catalog pair `reopen_outcome` (`data_fate`, entry `reopen_outcome`): FailClosed + WAL damage ⇒ `RefuseOpen`, never `ServeAll` (G8). AS-IS swallows damage and serves the damaged WAL. Production `wal/reopen_kernel.rs` is now the Verus term (`single_artifact`). Lemma `lemma_mutant_swallows_damage` is the FSCQ-class second possibility. Pair `dictionary_link` keeps the twin until its turn.

## Not claimed

CrashMonkey campaign over Pedra's Env. Dump of `db.rs` open glue. “somos seL4”. FSCQ's Coq spec.
