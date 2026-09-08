# Leave in the log is not done until it is committed

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
A committed change stays visible. Treating “in the log” as finished
(`queued_leave_finish_ok_as_is`) is the as-is mutant.

## Used this turn

Catalog pair `queued_leave_finish` (`data_fate`, entry
`queued_leave_finish_ok`): production `membership_kernel.rs` is already
the Verus term. This turn flags `single_artifact`. Plant
`queued_leave_finish_ok_on_live_uncommitted_leave_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `finish_uncommitted_leave`. “somos seL4”.
