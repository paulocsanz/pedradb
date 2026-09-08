# High-water is max(disk, RAM), not RAM/CLI length

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
A committed membership change on disk must stay visible. Forgetting
disk high-water (`high_water_at_least_as_is` returns RAM only) is the
as-is mutant.

## Used this turn

Catalog pair `high_water` (`data_fate`, entry `high_water_at_least`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`high_water_at_least_on_live_disk_ahead_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `open_single_node_with_rng_opts`. “somos seL4”.
