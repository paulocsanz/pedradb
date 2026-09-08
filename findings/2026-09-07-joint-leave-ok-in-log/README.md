# Leave-joint must be in the log before the config is single

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
The change must reach a quorum of Q₂. Skipping leave-joint
(`joint_leave_ok_as_is` always true) leaves C-old,new in force forever.

## Used this turn

Catalog pair `joint_leave_ok` (`data_fate`, entry `joint_leave_ok`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`joint_leave_ok_on_live_missing_leave_is_not_ok`. Other membership pairs
keep twins.

## Not claimed

Dump of `leave_joint`. “somos seL4”.
