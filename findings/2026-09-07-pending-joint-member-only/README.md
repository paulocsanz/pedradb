# Only current members define the pending joint

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 leftover `membChanging`
after peer-remove (already persisted under
`findings/2026-09-07-l28-tcp-clear-closed/`). Scanning opened nodes that
are no longer in `ids` is the as-is mutant
(`pending_joint_node_counts_as_is` always true).

## Used this turn

Catalog pair `pending_joint_node` (`data_fate`, entry
`pending_joint_node_counts`): production `membership_kernel.rs` is
already the Verus term. This turn flags `single_artifact`. Plant
`pending_joint_node_counts_on_live_non_member_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `pending_joint`. “somos seL4”.
