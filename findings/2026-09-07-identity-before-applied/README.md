# Persist C-new identity before advancing applied past the joint

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — already persisted
under `findings/2026-09-07-discard-uncommitted-local/openacid-raft-bug.md`).
A committed change stays visible. Persisting applied first (crash
window: applied high, voters stale) is the as-is mutant.

## Used this turn

Catalog pair `identity_before_applied` (`data_fate`, entry
`membership_identity_before_applied`): production
`membership_kernel.rs` is already the Verus term. This turn flags
`single_artifact`. Plant
`membership_identity_before_applied_on_live_applied_first_is_not_ok`.
Other membership pairs keep twins.

## Not claimed

Dump of `apply_range`. “somos seL4”.
