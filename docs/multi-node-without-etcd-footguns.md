# Multi-nó sem as footguns do etcd

**Status:** product doctrine (living)  
**Updated:** 2026-08-11  
**Product:** [MontanhaDb](montanhadb.md) coordination plane  
**Applies to:** `pedradb-raft` + `pedradb-dcs` (+ HTTP) — **not** the PedraDB kernel alone  
**Related:** [etcd comparison](etcd-comparison.md), [DCS for Patroni](pedradb-as-dcs-storage-for-patroni.md), [doctrine](doctrine-primitives-and-api-layers.md), [live leadership / Patroni-shaped HA](live-leadership-and-patroni-shaped-ha.md), [DCS market](dcs-market-landscape.md)

---

## Goal

**Multi-node coordination (CP)** for HA metadata: leader keys, config, members, watches —  
**without** the ops and API traps that make etcd painful in the wild.

We still use **Raft + one leader for writes**. That is not a footgun; that is how you get  
a single serial history. The footguns are what etcd (and operators) do **around** that.

---

## What we keep (non-negotiable)

| Choice | Why it is not a footgun |
|--------|-------------------------|
| One Raft leader proposes writes | Single total order; no multi-master reconciliation |
| Majority commit before “durable OK” | Same correctness class as etcd |
| Small metadata keyspace | Not a general DB; refuse to become one |
| PedraDB local engine per member | Clear process boundary; no shared directory multi-process |

**Leader is required for multi-node writes.** Eliminating the leader means eliminating  
linearizability (or inventing CRDTs for coordination — wrong tool for Patroni/K8s-class locks).

---

## etcd footguns → Pedra rule

### 1. “Everything is linearizable read → leader always”

**etcd:** Default reads go through the leader (or ReadIndex); people hammer reads and  
blame disk; then flip to serializable and get silent staleness.

**Pedra:**
- **Write:** leader only (explicit).  
- **Read:** API splits:
  - `get_local` / `dcs_get` on any node = **applied** state (may lag commit by design, documented).  
  - `get_linearizable` (future) = leader or ReadIndex — **opt-in**, never default for watch/UI.  
- Clients must **name** the consistency. No silent downgrade.

### 2. fsync tax + “slow disk kills the cluster” without feedback

**etcd:** Every commit is fsync-sensitive; latency spikes → election storms; little product  
guidance beyond “use fast disk”.

**Pedra:**
- **Default durable** (sync WAL) — honesty first.  
- **Group commit** as first-class: many Raft entries → one fsync barrier (API + metrics).  
- **Surface disk pain:** if fsync p99 > budget, **refuse writes with a typed error**  
  (`DiskTooSlow`) instead of flapping elections until the cluster is unusable.  
- **Never recommend NFS/shared block** for the data dir; reject/warn on known bad FS if detectable.

### 3. NOSPACE / quota alarm freezes the world

**etcd:** DB size exceeds quota → cluster-wide write stop; operators discover via alarm API.

**Pedra:**
- Soft quota with **early** warnings (metric + optional callback).  
- Hard stop only with **clear error** and **documented reclaim** (compact GC of DCS history,  
  not “delete random keys”).  
- Compaction is **automatic by default** for DCS revisions older than a watermark  
  (no “forgot to compact” time bomb). Default retention for coordination is short  
  (e.g. hours–days of history, not unbounded MVCC).

### 4. Unbounded MVCC / revision history

**etcd:** Revisions accumulate; watchers and space depend on compaction hygiene.

**Pedra DCS:**
- History is **not** a user product.  
- Keep **latest value + short tombstone window** for watches.  
- Long history = explicit “audit” product, not default DCS.

### 5. Lease footgun (forget keepalive → key vanishes mid-failover)

**etcd:** Lease expires → keys deleted; mis-tuned TTL causes split-brain app behavior.

**Pedra:**
- Leader lock APIs return **structured lease state** (TTL remaining, renew deadline).  
- **Min TTL floor** (e.g. ≥ 5s) and **max TTL ceiling** for coordination keys.  
- Optional **sticky lock**: expiry only after N missed renewals *and* new contender,  
  with documented fencing token (revision) so clients never trust “I still hold the key”  
  without checking revision.

### 6. Watch storms / lost events on reconnect

**etcd:** Clients reconnect wrong; miss events; duplicate handling is ad hoc.

**Pedra:**
- Watch is **cursor-based** (`since_revision` / log index). Reconnect = “from cursor”,  
  never “from now” by default.  
- Server may compact past cursor → **explicit `CursorGone`** (must full resync), never silent gap.

### 7. Member reconfiguration footguns

**etcd:** Wrong `member remove` / even cluster size / two clusters same token.

**Pedra:**
- Cluster membership changes are **joint consensus** (or single-node add/remove one at a time)  
  with **forced odd voter count** (3/5). Even size rejected.  
- **Cluster ID** in every RPC; mismatch = hard refuse (no silent merge).  
- Learners for new nodes until caught up (no vote until ready).

### 8. One shared Raft for “all of Kubernetes”

**etcd:** One group; noisy neighbor; huge DB.

**Pedra:**
- **One Raft group per coordination domain** (e.g. one Patroni cluster / one product).  
- Kernel stays embeddable; do not put app data in the DCS group.  
- Refuse oversized values (hard limit, e.g. 512KiB) with clear error.

### 9. Linearizability vs “I just want HA config”

**etcd:** One product, one consistency story, operators overload it.

**Pedra products:**
| Product | Consistency |
|---------|-------------|
| DCS locks / leader | Linearizable writes; fencing via revision |
| Config get for UI | Local applied OK |
| Embed single-node DCS | No Raft; no leader election tax |

### 10. Ops opacity

**etcd:** Many failure modes look like “timeout”.

**Pedra:** Typed errors + minimal status:
- `NotLeader { hint }`  
- `DiskTooSlow`  
- `QuorumLost`  
- `CursorGone`  
- `ValueTooLarge`  
- `ClusterIdMismatch`  

---

## Architecture we want (multi-nó)

```
        clients (Patroni / control plane / HTTP)
                        │
              ┌─────────┴─────────┐
              │  pedra-dcs API    │  explicit consistency per call
              └─────────┬─────────┘
                        │ only leader proposes
         ┌──────────────┼──────────────┐
         ▼              ▼              ▼
      node 1         node 2         node 3
      Raft +         Raft +         Raft +
      PedraDB        PedraDB        PedraDB
      (local)        (local)        (local)
```

- **No shared disk** between nodes.  
- **No multi-writer same Db directory.**  
- Replication = **Raft log of `DcsCommand`**, already started in-tree.

---

## What we deliberately will not copy from etcd

- Default linearizable read on every Get.  
- Unbounded revision history as the main data model.  
- Quota that bricks the cluster without a reclaim path.  
- Lease as a silent landmine without fencing tokens.  
- “Just fsync harder” without group commit and disk SLO errors.  
- One mega-cluster for all metadata of every product.

---

## Implementation status (facts)

| Item | Today |
|------|--------|
| Multi-node Raft + leader | ✅ `pedradb-raft` |
| DCS commands on Raft | ✅ `DcsCommand` + `propose_dcs` |
| Local applied read on any node | ✅ `dcs_get` |
| Explicit linearizable read API | ❌ not yet (do not add as silent default) |
| Group fsync for Raft batch | partial (core has `no_sync`+`sync`; Raft path still sync-heavy) |
| Auto compaction of DCS history | ❌ ( PedraDB GC exists; DCS history policy TBD ) |
| DiskTooSlow / typed disk errors | ❌ |
| Membership joint consensus | ❌ |
| Min/max TTL + fencing docs in API | partial (leases local; multi-node lease=0 first) |

---

## North star sentence

**Multi-node yes; etcd-shaped footguns no: leader for write order, honest durability,  
local reads by default, short history, typed failures, one coordination domain per cluster.**
