# Recover apply runs on every local replica, even if not in ids

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
Skipping recover-apply on a local replica that is no longer in `ids`
(`recover_apply_node_counts_as_is` requires `in_ids`) is the as-is
mutant.

## Used this turn

Catalog pair `recover_apply_node` (`data_fate`, entry
`recover_apply_node_counts`): production `membership_kernel.rs` is
already the Verus term. This turn flags `single_artifact`. Plant
`recover_apply_node_counts_on_live_local_non_member_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `recover_apply_committed`. “somos seL4”.
