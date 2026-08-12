# Applying logs into PedraDB (Raft sketch)

**RFC:** [0010-dbs-on-top](rfc/0010-dbs-on-top.md)  
**Crate:** `pedradb-apply`

## Contract

1. **Only the Raft leader** (or a single fake log) applies entries in index order.  
2. Each log entry payload deserializes to `Vec<BatchOp>`.  
3. `LogApplier::apply_entries` → `Db::apply_batch` (atomic multi-key, no OCC).  
4. After apply, export `db.snapshot().sequence()` as the state-machine watermark for snapshots / learners.  
5. Followers: either apply the same committed log, or (later) install SST/snapshot from leader.

## Multi-node catch-up (no real Raft yet)

`replicate_log_to_nodes(log, &mut [Db], &mut cursors)` applies the same
`FakeLog` to every open PedraDB directory — models follower catch-up after
commits land in a shared ordered log.

### `InProcessCluster` (P1.1 lite)

Educational FakeLog majority apply (no terms/elections).

### `pedradb-raft` (P1.1 real)

In-process **Raft** with terms, `RequestVote`, `AppendEntries`, commit index, and
apply into PedraDB via `LogApplier`:

```text
RaftCluster::open(dir, 3)
  → elect_leader(max_ticks)
  → propose_puts([(k,v), …])   // leader only
  → all nodes get() after commit
```

No network yet — RPCs are direct calls. Production networking can host the same
state machine or swap to OpenRaft while keeping `LogApplier` as the apply hook.
See [RFC-0012](rfc/0012-next-significant-steps.md) for multi-process Raft.

### DCS + Raft

`DcsCommand` (create/cas/put/delete) is logged as a single marker `BatchOp` and
applied via `apply_dcs_command` on every node. Leader checks preconditions before
propose. Network API: `PeerClient::propose_dcs` / `dcs_get`.

**Live leadership (best-effort open sessions), fencing, and Patroni-shaped HA** are
specified in [live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md).

### WAL-shipped read replica (P1.2)

Crate **`pedradb-replicate`** ships **physical** `CURRENT.log` bytes:

```text
primary puts (no flush) → WalShipper::pull → append_wal_bytes(replica)
→ Db::open(replica) recovers same sequences
```

Flush on the primary rotates the WAL → `ShipError::WalRotated` (re-bootstrap).
Not a substitute for Raft; use for async read replicas of one writer.

### `KvService` (P1.3)

In-process get/put/delete/begin/apply façade over `Db`. Wire (gRPC) later.

## Minimal loop (pseudo)

```text
on_commit(entries):
  applier.apply_entries(entries)
  // optional: if mem large, Db auto-flush (RFC-0009) already may run

on_snapshot_request:
  return (applier.snapshot().sequence(), last_log_index)
```

## What PedraDB does not do

- Leader election, log replication, membership — stay in the Raft library.  
- Network server — outer product.  
- Cross-region 2PC — Rung 4+ of the grail.

## Engine maturity (RFC-0009)

For production Raft state, prefer:

- Auto-flush on (default 4 MiB) or explicit flush after apply batches.  
- Periodic `compact` under size pressure.  
- Group fsync: `WriteOptions::no_sync` on apply + barrier `db.sync()` once per Raft batch of entries if needed.
