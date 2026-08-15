# Montanha read capacity — strong vs replica (RFC-0025 P2.3)

**Status:** shipped API + policy  
**Updated:** 2026-08-14  

## Policies

| API | Policy | Linearizable? | When to use |
|-----|--------|---------------|-------------|
| [`get`](../crates/pedradb-store) | LocalApplied on local Pedra | **No** (may lag) | Default local path; highest QPS |
| [`get_strong`](../crates/pedradb-store) | Live **range leader** only | **Yes** (leader applied) | FDB-class “read your writes” / strict SI consumers |
| [`get_fast_replica`](../crates/pedradb-store) | Best **follower** applied (else leader) | **No** | Scale-out RO; dashboards; etcd-need watchers that tolerate lag |
| [`get_with_policy`](../crates/pedradb-store) | Explicit [`ReadPolicy`] | Depends | Custom routing |

## Ergonomics

```rust
// Strict
let v = cluster.get_strong(b"k")?;

// Cheap / scale-out RO
let v = cluster.get_fast_replica(b"k")?;

// Lag observability
let lag = cluster.applied_lag(follower_id, range_id);
```

## Capacity notes vs peers

| Peer | Read scale model | Montanha analogue |
|------|------------------|-------------------|
| **FDB** | Proxies + storage; GRV for snapshot | `get_strong` ≈ read at leader after commit; snapshots via TX |
| **TiKV** | Follower read / stale read | `get_fast_replica` + `applied_lag` |
| **etcd** | linearizable vs serializable | Strong vs LocalApplied |

**Rule:** layers that need FDB-like correctness after a concurrent writer must use **TX** or **`get_strong`**, not bare `get`.

## Tests

- Dual-leader / deposed leader: Strong fails closed (`get_with_policy` tests in store).  
- Follower LocalApplied still serves (by contract).  

## Out of scope

- Wire-level strong read over TCP (today: client dials leader via NotLeader retry for puts; strong get still needs leader id from status or future wire tag).  
- Bounded-staleness leases (TiKV-style) — P2 later.  
