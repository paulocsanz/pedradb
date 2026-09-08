# Wire grant only after votedFor is durable

**Date:** 2026-09-07
**Primary sources:**
- Ongaro/Ousterhout Raft Fig. 2: `votedFor` is persistent state, updated
  on stable storage **before responding to RPCs**.
- Same Databend 2025-10-11 IO-order note already persisted under
  `findings/2026-09-07-durable-term-persist-before-act/`: do not answer
  at a term/vote that has not hit disk.

## What the source actually says

A WouldGrant decision that then fails persist must not become
`voteGranted` on the wire. The as-is mutant ignores persist.

## Used this turn

Catalog pair `grant_persist` (`data_fate`, entry `grant_after_persist`):
Verus `g ==> persist == Ok` on production `vote_kernel.rs`. rustc body
last-wins. Pair `vote` keeps its twin.

## Not claimed

Dump of `handle_request_vote_with_persist`. “somos seL4”.
