# Joint-remove target is the membership set, not local nodes

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 leftover membership
after peer-remove (already persisted under
`findings/2026-09-07-l28-tcp-clear-closed/`). Requiring the target in
local `nodes` (TCP replica only has self) is the as-is mutant.

## Used this turn

Catalog pair `joint_target` (`data_fate`, entry `joint_target_counts`):
production `membership_kernel.rs` is already the Verus term. This turn
flags `single_artifact`. Plant
`joint_target_counts_on_live_ids_not_nodes_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `remove_member_joint`. “somos seL4”.
