# A removed node is not participating

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 leftover membership
after peer-remove (already persisted under
`findings/2026-09-07-l28-tcp-clear-closed/`). Keeping captured
participating after leave (`participating_if_member_as_is` always true)
is the as-is mutant.

## Used this turn

Catalog pair `participating_member` (`data_fate`, entry
`participating_if_member`): production `membership_kernel.rs` is already
the Verus term. This turn flags `single_artifact`. Plant
`participating_if_member_on_live_removed_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `is_participating`. “somos seL4”.
