# Term rises only when hard state is durable

**Date:** 2026-09-07
**Primary sources:**
- Ongaro/Ousterhout Raft (Fig. 2): persistent state on all servers is
  *updated on stable storage before responding to RPCs* (`currentTerm`,
  `votedFor`, `log[]`).
- Databend 2025-10-11 *Raft 中的 IO 执行顺序* (SegmentFault
  `1190000047314653`, persisted as `databend-raft-io-order.md`): in an
  implementation that splits memory `current_term` from disk
  `persisted_term`, the process must not act at a term that has not hit
  disk. Precisely: wait until `persisted_term >= req.term` before Ok.

## What the source actually says

A newer incoming term that failed to persist must leave the process on
the previous term. Raising the in-memory term and then answering RPCs
at that term is the as-is mutant (`durable_term_if_newer_as_is`).

## Used this turn

Catalog pair `durable_term` (`data_fate`, entry `durable_term_if_newer`):
Verus `term_rises_only_when_durable` + `restored_keeps_previous_term` on
production `crates/pedradb-raft/src/vote_kernel.rs`. rustc body stays
last-wins (clone `vote_raft_store` token-identical). Pairs `vote` /
`grant_persist` keep twins.

## Not claimed

IO-reorder of save-term vs save-entries. Dump of
`handle_request_vote_with_persist`. “somos seL4”.
