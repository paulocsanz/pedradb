# Removing a peer must close leftover local state, not leave membership in progress

**Date:** 2026-09-07
**Primary source:** nats-io/nats-server PR #7609 (merged 2025-12-04),
`nats-pr-7609.md` in this directory if fetched. A node that removes all
followers was left with `membChanging` set permanently — leftover
membership state blocked later admits.

## What the source actually says

After a peer-remove, leftover local flags must be cleared. A stuck
in-progress bit is the as-is mutant (`l28_tcp_clear_ok_as_is` ignores
the close).

## Used this turn

Catalog pair `l28_tcp_clear` (`data_fate`, entry `l28_tcp_clear_ok`):
production `l28.rs` is already the Verus term. This turn flags
`single_artifact`. Plant `l28_tcp_clear_ok_requires_closed`.

## Not claimed

Invented `l28_tcp_add`. Dump of `cluster_real`. “somos seL4”.
