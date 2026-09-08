# Recover must apply when commit is ahead of applied

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
A committed change stays visible. Skipping apply on recover (committed
joint stays C-old) is the as-is mutant.

## Used this turn

Catalog pair `recover_apply` (`data_fate`, entry `recover_must_apply`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`recover_must_apply_on_live_commit_ahead_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `recover_apply_committed`. “somos seL4”.
