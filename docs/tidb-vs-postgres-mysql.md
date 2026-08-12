# TiDB vs Postgres vs MySQL

**Status:** comparison (complements `tidb-architecture.md`, which is TiDB internals vs TiKV/FDB/CRDB)  
**Updated:** 2026-08-11  
**Related:** `sql-lessons-for-the-grail.md`, `grail-plan-build-databases-on-pedradb.md`, `plug-map-replace-incumbents.md`

---

## TL;DR

**Postgres** and **MySQL** are single-node monoliths: one process, one primary writer, scale vertically and via **read** replicas.  
**TiDB** is a distributed system that speaks the **MySQL wire**: stateless SQL on top of **TiKV** (multi-Raft + Percolator) with **PD** as placement/TSO brain.

Trade: TiDB buys **horizontal write scale** and **native HA**, paying **per-query network latency**, **compatibility quirks**, and **cluster ops cost**.

---

## Side-by-side

| | **Postgres** | **MySQL (InnoDB)** | **TiDB** |
|--|--------------|--------------------|----------|
| **Architecture** | Monolith; heap + B-tree; MVCC + vacuum | Monolith; clustered B-tree; MVCC via undo | Compute/storage split: TiDB (stateless) + TiKV (Regions/Raft) + PD (TSO/schedule) |
| **Write scale** | 1 primary | 1 primary (Group Replication multi-primary exists, rarely the default path) | **N region leaders** — horizontal write scale; single-writer **per key range** |
| **Isolation** | RC default; real **SSI** available | RR default + gap locking | SI labeled “Repeatable Read”; no gap locks; not strict serializable |
| **Distributed TX** | N/A (or manual/FDW/XA-shaped) | N/A (or fragile XA) | Native Percolator 2PC; pessimistic default in modern TiDB |
| **Unit latency** | µs–ms, no RPC | Same | Always pays network: TSO + gRPC + Raft quorum. Wins aggregate throughput; loses single-query latency |
| **HA** | Streaming replication + orchestrated failover (Patroni, etc.) | Binlog async/semi-sync + orchestrated failover | Native Raft per Region; automatic failover; RPO≈0 with quorum |
| **HTAP** | Replicas + external tools | HeatWave (Oracle Cloud niche) | TiFlash (columnar Raft learner) |
| **Ops** | One process — simple | One process — simple | Serious min cluster ~3 PD + 3 TiKV + 2 TiDB. Rarely worth it below hundreds of GB / high write |
| **Ecosystem** | Richest SQL (extensions, types, FDW, transactional DDL) | Ubiquity, mature binlog tooling | MySQL drivers/ORMs; gaps: triggers/procs historically, FKs late/limited, non-contiguous auto-increment |

---

## How to choose

| Situation | Prefer |
|-----------|--------|
| Default app DB, rich SQL, real serializable, up to ~TB on a good node | **Postgres** |
| Already MySQL shop, binlog ops, team trained, InnoDB predictability | **MySQL** |
| Wrote past single-writer MySQL; alternative is manual sharding / Vitess | **TiDB** (or Vitess if you accept no live reshard + app-aware shards) |
| Need Postgres wire + horizontal writes | **Not TiDB** — CockroachDB / Yugabyte / Citus (different products). There is no “TiDB for Postgres” from PingCAP. |

**Postgres limits:** one writer; vacuum under heavy churn.  
**MySQL limits:** feature race largely lost to PG; multi-primary is not the main story.  
**TiDB limits:** latency tax, ~8-node floor for serious HA, MySQL “almost” compatibility bites.

---

## Asymmetry

```
MySQL wire  ──►  TiDB (distributed)
Postgres wire ──► CockroachDB / Yugabyte / Citus  (not TiDB)
```

TiDB is the distributed **MySQL-shaped** product. Postgres-shaped horizontal is a **different** product line.

---

## Tie-in to PedraDB

| System | Where TX lives | Distribution | PedraDB lesson |
|--------|----------------|--------------|----------------|
| Postgres / MySQL | **In the monolith core** (correct for single node) | None (replicas are log shipping) | TX-in-core is right; they never scale writes without rewriting storage/distribution |
| TiDB | **Bolted on** RocksDB via Percolator | Multi-Raft + PD | Exactly the tax PedraDB refuses for the kernel: mute local engine + years of distributed TX |
| PedraDB grail | TX in **local** kernel | Outer multi-Raft product | Target quadrant: **embed TX** *and* grow to N writers via region leaders — neither pure monolith nor bolt-on-only |

**Recipe P** (horizontal Postgres/SQL on multi-Raft + PedraDB) aims at the quadrant **none of the three occupy today**:

- Not “monolith forever” (PG/MySQL ceiling)  
- Not “SQL on mute RocksDB” (TiDB tax)  
- Instead: **local multi-key ACID** (PedraDB) → **multi-Raft** → **SQL wire** as a layer  

Alternate strategies (documented, not blind spots — see `sql-lessons-for-the-grail.md`):

| Strategy | Example | vs Recipe P |
|----------|---------|-------------|
| (a) Own SQL + own dist KV | TiDB, CRDB | Grail path |
| (b) Unmodified SQL engine + swapped storage | Aurora, Neon | Cheaper; couples to PG/MySQL license/engine |
| (c) Proxy + shard unmodified engines | Vitess, Citus | Cheaper; fixed shards / manual reshard |

---

## Short-term PedraDB impact

**None on P0 scope.** This comparison validates:

1. Keep kernel **local + TX** (monolith lesson).  
2. Don’t put MySQL/PG wire in core (TiDB surface lives above).  
3. Don’t start with multi-Raft before a working local TX (TiDB’s stack order: storage first).  
4. When we *do* distribute later: single-writer-per-range (TiDB/CRDB), not multi-master same key.

---

## Sources

- In-repo: `tidb-architecture.md`, `sql-lessons-for-the-grail.md`, `distribution-deep-research.md`  
- Industry defaults: Postgres SSI / streaming replication; MySQL InnoDB RR + binlog; TiDB/PD/TiKV docs (see tidb-architecture sources)
