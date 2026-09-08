# Joint-add target need not live in local nodes

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
The joining process is another OS pid. Requiring it in local `nodes`
(`joint_add_target_counts_as_is`) is the as-is mutant.

## Used this turn

Catalog pair `joint_add_target` (`data_fate`, entry
`joint_add_target_counts`): production `membership_kernel.rs` is already
the Verus term. This turn flags `single_artifact`. Plant
`joint_add_target_counts_on_live_not_in_nodes_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `add_member_joint`. “somos seL4”.
