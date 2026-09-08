# Durable membership overrides CLI --peer on open

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 leftover membership
state after peer-remove (already persisted under
`findings/2026-09-07-l28-tcp-clear-closed/`). Letting CLI `--peer`
overwrite non-empty disk membership is the as-is mutant
(`disk_membership_overrides_cli_as_is` always false).

## Used this turn

Catalog pair `disk_membership` (`data_fate`, entry
`disk_membership_overrides_cli`): production `membership_kernel.rs` is
already the Verus term. This turn flags `single_artifact`. Plant
`disk_membership_overrides_cli_on_live_has_disk_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `bind_cluster_identity`. “somos seL4”.
