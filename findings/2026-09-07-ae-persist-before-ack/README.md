# Persist dirty log before AE `success: true`

**Date:** 2026-09-07
**Primary sources:**
- Bendersky, *Implementing Raft: Part 3 — Persistence and Optimizations*
  (page still live 2026-08-28). Raft Figure 2: persistent state is written
  and flushed before the server issues the next RPC **or replies to an
  ongoing RPC**. Excerpt: `eli-raft-persist.txt`.
- FVSquad raft-lean-squad REPORT (2026-04-28), persistence chain
  MaybePersist / MP6: async-persist never advances past stable storage.
  Already on disk: `findings/2026-09-07-raft-ae-log-matching/REPORT.md`.

Park (KTH 2025) “persist log entries before acknowledgment” was named in
search; the PDF fetch to `kth.diva-portal.org` timed out. Not treated as
evidence.

## Used this turn

Catalog pair `ae_ack` (`data_fate`, entry `ae_ack_success`): dirty log ⇒
`success` only if `persist_ok`. AS-IS always acks. Production
`ae_kernel.rs` is the Verus term (`single_artifact`). Lemma
`lemma_success_reply_only_after_persist` is the second possibility. Pair
`ae_f16_gate` keeps the twin until its turn.

## Not claimed

etcd’s every-entry fsync. Dump of `handle_append_entries` persist glue.
“somos seL4”.
