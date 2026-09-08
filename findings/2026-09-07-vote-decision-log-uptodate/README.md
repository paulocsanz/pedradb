# RequestVote grants only on matching term, free vote, and log up-to-date

**Date:** 2026-09-07
**Primary source:** Ongaro/Ousterhout Raft §5.2 / §5.4.1 (Fig. 2
RequestVote receiver): grant iff `votedFor` is null or candidateId **and**
the candidate’s log is at least as up-to-date (last log term, then index).

## What the source actually says

A matching term is not enough. The as-is mutant
`vote_decision_as_is_ignore_log_and_vote` grants on term match alone.

## Used this turn

Catalog pair `vote` (`data_fate`, entry `vote_decision`): Verus
`should_grant` iff on production `vote_kernel.rs` (flattened u64
stand-in; rustc `VoteInputs` body last-wins).

## Not claimed

Dump of `handle_request_vote`. “somos seL4”.
