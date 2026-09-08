# Persist-leader after discard must be a local node

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 leftover membership
state after peer-remove (already persisted under
`findings/2026-09-07-l28-tcp-clear-closed/`). A remote `ids.first()`
as persist-leader is the as-is mutant.

## Used this turn

Catalog pair `discard_leader` (`data_fate`, entry `discard_leader_local`):
Verus on production `crates/pedradb-raft/src/membership_kernel.rs`.
rustc body last-wins (store clone token-identical). Plant
`discard_leader_local_on_live_remote_is_not_ok`. Other membership
pairs keep twins.

## Not claimed

Dump of `finish_queued_propose`. “somos seL4”.
