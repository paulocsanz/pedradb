# C-old,new stays in force while the sets differ

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
Joint consensus’s middle state is the product of the two majority sets.
Treating every config as single (`joint_still_active_as_is` always
false) is the as-is mutant — leave-joint never runs.

## Used this turn

Catalog pair `joint_leave` (`data_fate`, entry `joint_still_active`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`joint_still_active_on_live_differing_sets_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `pending_joint_on`. “somos seL4”.
