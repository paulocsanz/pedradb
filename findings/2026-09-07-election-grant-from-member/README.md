# Vote grants count only from current or pending members

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
A lagging removed voter is not in C-old or C-new. Recording that grant
(`election_grant_from_counts_as_is` always true) is the as-is mutant.

## Used this turn

Catalog pair `election_grant_from` (`data_fate`, entry
`election_grant_from_counts`): production `membership_kernel.rs` is
already the Verus term. This turn flags `single_artifact`. Plant
`election_grant_from_counts_on_live_neither_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `on_request_vote_reply`. “somos seL4”.
