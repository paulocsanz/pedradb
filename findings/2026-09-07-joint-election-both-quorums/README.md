# Joint election needs majority of C-old and C-new

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
Joint consensus’s middle quorum is M(C-old)×M(C-new). Electing on C-old
alone while a joint add is in flight is the as-is mutant.

## Used this turn

Catalog pair `joint_election` (`data_fate`, entry `joint_election_ok`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`joint_election_ok_on_live_old_only_is_not_ok`. Other membership pairs
keep twins.

## Not claimed

Dump of `election_has_joint_quorum`. “somos seL4”.
