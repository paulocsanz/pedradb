# Uncommitted membership must stay recognizable as uncommitted

**Date:** 2026-09-07
**Primary source:** OpenACID, *The Pitfalls of Raft Membership Change*
(https://blog.openacid.com/distributed/raft-bug/ — fetched 2026-09-07;
full text in `openacid-raft-bug.md`).

## What the source actually says

A committed change stays visible. If one change is already committed,
every uncommitted change must be recognizable as uncommitted. Otherwise
a new leader cannot tell which of them to keep. Skipping discard of the
uncommitted suffix on a local replica that is no longer in `ids` is the
as-is mutant (`discard_node_counts_as_is` requires `in_ids`).

## Used this turn

Catalog pair `discard_uncommitted` (`data_fate`, entry
`discard_node_counts`): production `membership_kernel.rs` is already the
Verus term. This turn flags `single_artifact`. Plant
`discard_node_counts_on_live_local_non_member_is_not_ok`. Other
membership pairs keep twins.

## Not claimed

Dump of `discard_uncommitted_from`. “somos seL4”.
