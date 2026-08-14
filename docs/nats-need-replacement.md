# Replacing the *need* for NATS (esp. after Jepsen JetStream)

**Status:** architecture note (not a product commitment)  
**Updated:** 2026-08-11  
**Related:** `scylla-need-replacement.md` (same “need vs product” pattern),  
`sql-lessons-for-the-grail.md` (log-is-the-DB),  
`conversation-learnings-and-short-term-alignment.md`,  
`grail-plan-build-databases-on-pedradb.md`

Primary source: [Jepsen: NATS 2.12.1](https://jepsen.io/analyses/nats-2.12.1) (Kyle Kingsbury, 2025-12-08).  
Synadia response: [blog](https://www.synadia.com/blog/jepsen-nats-2-12-1).  
Context: Railway mono runs NATS with **JetStream explicitly disabled** (`platform/nats/values.yaml`) — core pub/sub only.

---

## 1. Two different NATS products

| | **Core NATS** | **JetStream** |
|--|---------------|---------------|
| Role | Pub/sub, request-reply, fanout | Durable streams, consumers, replay, acks |
| Delivery | At-most-once / best-effort; no long-term store | Claims **at-least-once** for acknowledged publishes |
| Persistence | Ephemeral (connected subscribers only) | Disk + **Raft** clustering |
| Docs claim (JetStream) | — | “Linearizable” + “self-heal and always available” (Jepsen notes these cannot both be absolute) |
| What Jepsen attacked | Not the main focus | **JetStream 2.12.1** durability / split-brain under faults |

**Question people mean by “replace NATS” is almost always either:**

1. **Messaging bus** (Core) — low-latency fanout, no durable history; or  
2. **Durable log / queue** (JetStream) — ordered stream that survives crashes, consumers with offsets/acks.

These need **different** answers on PedraDB.

---

## 2. What Jepsen found (JetStream 2.12.1) — critical shape

Summary from the analysis (not a full rehash):

| Finding | Shape of failure |
|---------|------------------|
| **Lazy `fsync` by default** | Writes flushed on a timer (~2 minutes), **not** before ack of publish → LazyFS / power-loss can drop **acknowledged** messages |
| **`.blk` corruption (minority nodes)** | Bitflip/truncation → large windows of **lost** acked writes; sometimes **split-brain** (different nodes return different message sets) |
| **Snapshot corruption** | Node with corrupt snapshot can become metadata leader and **delete** stream as “orphan”; quorum never heals |
| **Earlier 2.10.20–22** | Process crashes alone could **wipe entire stream** (reported fixed in 2.10.23) |
| **Doc vs physics** | “Linearizable” and “always available” cannot both hold in the absolute sense (CAP) |

Jepsen did **not** primarily claim “pub/sub is broken.” It claimed: **JetStream’s durable commit story fails under corruption and power-loss class faults**, partly because **ack ≠ durable on disk**.

That maps **directly** onto PedraDB’s durability debate (O1): default `DataSync` before `commit` returns Ok is the opposite of JetStream’s default timer flush.

---

## 3. Can a DB on PedraDB replace NATS?

### 3.1 JetStream-class durable stream — **yes, as architecture** (not drop-in protocol)

This is a **log product**, not “SQL.” Same family as Kafka / Redpanda / “WAL is the product”:

```
Producers ──append──►  multi-Raft log (one leader per stream/shard)
                              │
                              │ apply
                              ▼
                     PedraDB (or SST of log segments) per node
                              │
Consumers ──read/subscribe──►  offset cursor + push/pull API
```

| JetStream need | PedraDB-stack answer |
|----------------|----------------------|
| Totally ordered stream | Ordered keys `/stream/{id}/{seq}` or pure Raft log + PedraDB apply |
| At-least-once after ack | **Ack only after Raft quorum + PedraDB durable commit** (sync policy explicit) |
| Consumer offsets | Keys `/consumer/{id}/offset` multi-key TX with optional dedupe |
| Replay from offset | Range scan / log read from seq |
| HA | Multi-Raft RF=3/5; single leader per stream range |
| Avoid split-brain | CP: only leader accepts appends; followers don’t invent alternate histories |
| Corruption | Checksums + **heal from majority** (don’t elect leader that deletes majority-committed data) |

**PedraDB’s role:** local durable ordered KV + multi-key TX (offset + metadata in one commit).  
**Not PedraDB alone:** network, Raft, subject routing, NATS wire protocol, millions of ephemeral subscribers.

**Honest product name:** “durable stream / work queue on multi-Raft+PedraDB” — **not** “NATS-compatible.”

### 3.2 Core NATS (stateless pub/sub) — **no, not as a database**

Core NATS is optimized for:

- µs–ms fanout to many connected clients  
- No durable history  
- Request-reply  
- Subject wildcards, queue groups  

A **database** (even on PedraDB) that persists every message will **lose** on pure latency/throughput of fire-and-forget pub/sub. Replacing Core NATS with “put every message in a DB” is usually the wrong tool.

| If you need… | Prefer |
|--------------|--------|
| Ephemeral events, service mesh signals, best-effort | Keep Core NATS / Redis pubsub / similar **or** in-process channels |
| Durable work queue / audit log / replay | Log product on PedraDB+Raft (JetStream *job*) |
| Both | Split: bus for ephemeral + log for durable (many platforms already do this) |

**Railway mono today:** NATS with **JetStream disabled** → they are using the **Core** path. Jepsen’s JetStream findings are a warning **if** someone turns durability on or picks JetStream for “we need messages to survive.” They are **not** proof that Core NATS is the same failure mode.

### 3.3 Hybrid (common real design)

```
Hot path:  Core NATS / gRPC stream  (ephemeral fanout)
Cold path: durable log on PedraDB+Raft  (replay, workers, exactly-once-ish with consumer TX)
```

Or: **only** durable log if you don’t need Core’s latency (many “use NATS for jobs” shops only needed JetStream).

---

## 4. What PedraDB already gives you for a JetStream-shaped product

| Primitive | Status in PedraDB | Role in stream product |
|-----------|-------------------|-------------------------|
| Monotonic `SequenceNumber` | P0.2 done | Stream offset / LSN |
| Ordered keys + range | P0 goal | Read from offset |
| Multi-key ACID | P0.4 | Consumer offset + payload metadata atomic |
| WAL + sync commit default | P0.1 done; O1 locked | Opposite of JetStream’s 2‑minute fsync default |
| WAL export / apply_batch | P1.6 / P2 | Ship log, outer Raft apply |
| Multi-Raft cluster | Outer product | JetStream’s Raft layer replacement |

**Jepsen-shaped requirements that must live in the outer product, not only PedraDB:**

1. Never ack append until **quorum + durable** (document; test with LazyFS-class faults).  
2. Leader election must not promote a node that will **delete** majority-committed history.  
3. Corruption: detect + **repair from peers**, don’t silently truncate committed prefix.  
4. Jepsen-style tests for the **stream product** (not only unit tests on MemTable).

PedraDB kernel correctness is necessary but **not sufficient** — JetStream’s bugs were in **cluster + filestore + fsync policy**, exactly the outer layer.

---

## 5. Comparison to Scylla *need* replacement

| | Scylla need (mono networking) | NATS / JetStream need |
|--|------------------------------|------------------------|
| Data | Routes, WID→host, assignments | Ordered messages + consumer state |
| Scale-out | Many keys, many leaders | Many streams/shards, one leader per stream range |
| Push | Watch / CDC | Consumer fetch or push from log |
| Failure mode people fear | Stale route / blackhole | Lost acked message / split history |
| PedraDB fit | Ordered KV + TX | Ordered log as KV (or log-primary) + TX for offsets |

Same doctrine: **replace the job**, not the wire protocol.

---

## 6. Short-term PedraDB conflict?

| Question | Answer |
|----------|--------|
| Does NATS research change P0? | **No** — still ship local TX + durable WAL |
| Should P0 implement streams? | **No** — different product |
| Does it strengthen O1 (sync default)? | **Yes** — Jepsen is a live case study of ack-before-fsync |
| Does it strengthen WAL-as-first-class artifact? | **Yes** — same as Aurora/Neon lessons |
| Drop-in NATS clients? | **Out of scope** forever unless a dedicated wire layer is funded |

---

## 7. Decision-shaped summary

| Claim | Verdict |
|-------|---------|
| “PedraDB replaces NATS” | **False** (PedraDB is a library) |
| “A multi-Raft + PedraDB **stream product** can replace **JetStream’s job**” | **True as architecture**, with honest durability + Jepsen-class testing |
| “That product is a NATS drop-in” | **False** unless you invest in protocol compatibility |
| “A DB on PedraDB replaces **Core** NATS pub/sub” | **Usually false** — wrong latency/semantics; keep a bus or accept different product |
| “Jepsen means never use NATS Core” | **Overclaim** — report targets JetStream durability; Core is a different contract |
| “Jepsen means JetStream defaults are unsafe for ‘acked = durable’” | **Fair reading of the report** for 2.12.1 defaults / corruption paths |

---

## 8. If we ever build this (ladder slot)

Suggested placement (does not reorder P0):

```
Rung 0   PedraDB
Rung 1.5 WAL-shipped replicas (optional)
Rung 3   multi-Raft
Rung 3.5 pedra-stream   ← JetStream-class: append/read/consumer offsets
Rung 6   watches (etcd-class; different API)
```

Subjects, queue groups, leaf nodes, superclusters = product surface on top — not kernel.

---

## Bottom line

1. **JetStream need** (durable ordered log, at-least-once after true durable ack) is a **natural outer product** on PedraDB + multi-Raft — and Jepsen shows *why* default async flush + weak corruption handling is fatal.  
2. **Core NATS need** (ephemeral high-speed bus) is **not** solved by “a database”; don’t force PedraDB there.  
3. **No drop-in.** Same rule as Scylla: replace the **job**, not the logo.  
4. **P0 unchanged** — but durability and log-export priorities stay **confirmed**, not diluted.

**Follow-up (2026-08-14):** the *consumer* of a bounded KV log (cursor-after-apply, expiry resync, fold export) is documented from Slipstream + Quicksilver v2 in [`slipstream-and-quicksilver-learnings.md`](slipstream-and-quicksilver-learnings.md). That layer is not JetStream and not Montanha TX.
