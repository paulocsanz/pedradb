# Truncated Raft log must not look committed (NATS #7587)

**Date:** 2026-09-07
**Primary source:** Van Veen, *NRG: WAL requires repair after truncation*, nats-io/nats-server#7587 (Nov 2025; still open as of fetch). Text: this README (GitHub HTML was the reachable source; no separate PDF).

## What the issue actually does

NATS Raft assumed the WAL was never corrupted. Truncation then let a node recover a *shorter* log and still treat it as the committed prefix, so state diverged. The fix: empty/truncated log is marked needing repair (`repair.idx`); the node is not fully operational until caught up. A majority of truncated nodes still halt until a complete log exists.

## Used this turn

Catalog pair `commit_raft` (`data_fate`, entry `propose_ack_ok`): client `Ok(index)` only if `commit_index >= index` (F11). AS-IS acks on append. Same file holds F10 `recover_commit = min(loaded, log_last)` — never `log.len()` as commit after a truncated reopen. Production `commit_kernel.rs` is now the Verus term. Lemma `lemma_as_is_acks_uncommitted` is the F11 second possibility.

## Not claimed

NATS repair.idx protocol. Dump of `raft/src/lib.rs` persist glue. “somos seL4”.
