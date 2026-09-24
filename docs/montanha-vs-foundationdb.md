# Montanha vs FoundationDB — what is the same, what is different, and why it confuses

**Status:** alignment note (read this when “FDB vs TiKV vs Montanha” feels muddy)  
**Updated:** 2026-08-13  
**Product:** [MontanhaDb](montanhadb.md)  
**Normative contract:** [RFC-0013](rfc/0013-montanhadb-product.md)  
**Related:** [fdb-limitations-analysis.md](fdb-limitations-analysis.md), [foundationdb-layers-and-products.md](foundationdb-layers-and-products.md), [montanha-layering-dcs-on-store.md](montanha-layering-dcs-on-store.md)

---

## 0. The one-paragraph answer

**Montanha is meant to be *like FoundationDB in product philosophy*, not a clone of FoundationDB’s internal machines, and not “a TiKV rewrite.”**

- **Like FDB:** one serious **ordered KV + transactions** substrate; everything richer (DCS, SQL, HA agents, records) is a **layer** on that substrate; locks and elections are **data + TX**, not a second etcd-shaped product.
- **Not FDB yet (and may never copy every knob):** FDB’s unbundled commit path (proxies, resolvers, transaction logs, storage servers), 5s TX timeout culture, Apple-scale simulation, decades of ops.
- **Where TiKV language snuck in:** multi-Raft *ranges* are one **engineering tool** for replicating and scaling writes. That is *how some systems move bytes*, not the *product identity*. Product identity is **FDB-shaped (layers + TX core)**.

If you remember only this:

| Layer of thinking | Montanha target | Closest cousin |
|-------------------|-----------------|----------------|
| **What is the product?** | Transactional KV core + layers | **FoundationDB** |
| **How might we replicate shards?** | Ranges / groups (today: multi-Raft MVP) | *Implementation*, often compared to TiKV |
| **What is DCS?** | A **layer** on the core | FDB app / layer — **not** etcd-as-identity |

---

## 1. Two different questions people mix up

People say “compare Montanha and FDB” and accidentally answer **three** different questions:

| # | Question | Good answer shape |
|---|----------|-------------------|
| **A** | *What kind of product is it?* | Substrate KV + TX; layers on top → **FDB-like** |
| **B** | *How does a write get majority-durable across machines?* | Consensus / logs / leaders → **implementation** (FDB ≠ TiKV here) |
| **C** | *How mature is it?* | FDB = production giant; Montanha = early MVP on PedraDB |

**Confusion recipe:**  
Someone answers B with “multi-Raft like TiKV,” and the listener thinks the answer to A is “we are a TiKV.”  
**Fix:** keep A fixed as FDB-shaped; treat B as plumbing that can evolve.

---

## 2. Same mountain shape (the FDB lesson we *do* copy)

```text
                    YOUR APPS
           (SQL, HA agents, locks, business)
                         │
                         ▼
              ┌─────────────────────┐
              │   LAYERS (libraries │
              │   or thin services) │
              └──────────┬──────────┘
                         │ transactions / ordered keys
                         ▼
              ┌─────────────────────┐
              │  ORDERED KV + TX    │  ← the “mountain base”
              │  (the real product) │
              └─────────────────────┘
```

| Idea | FoundationDB | Montanha / PedraDB |
|------|--------------|--------------------|
| Core is not SQL | Yes — KV + TX | Yes — PedraDB locally; Store as distributed substrate |
| Rich models are layers | Record Layer, Document Layer, company-private layers | DCS, SQL crate, stream, future record-ish layers |
| Coordination is not a second DB brand | Locks = keys + TX | DCS create/cas = keys on the store |
| “Don’t become etcd-the-product” | FDB is general; etcd is coord-specialized | Explicit anti-etcd identity ([layering doc](montanha-layering-dcs-on-store.md)) |
| Embeddable vs distributed | FDB is **distributed cluster** | **PedraDB** = embed kernel; **Montanha** = cluster product *on* that kernel |

This is why early docs said PedraDB follows FDB’s **design model** (ACID in the core, layers above) — see [fdb-limitations-analysis.md](fdb-limitations-analysis.md).

---

## 3. Where they actually differ (nuances)

### 3.1 One process vs a cluster (PedraDB vs FDB)

| | **PedraDB** | **FoundationDB** |
|--|-------------|------------------|
| Deploy | Library in *your* process | Many cooperating server processes |
| TX path | Function call into engine | Client → proxy → resolver → logs → storage (many hops) |
| Classic FDB limits (5s TX, 10MB TX, 100KB value) | **Do not apply** the same way — no remote resolver | Hard limits from distributed design |
| Role | Local **kernel** (like “storage engine + local TX”) | Full **distributed database** |

**Nuance:** PedraDB is *not* “FDB in-process.” It is the **local engine** Montanha peers use, closer to “what RocksDB/SQLite is under a distributed system” than to “all of FDB.”

```text
FDB client  ──network──►  FDB cluster (many roles)

Your app    ──function──►  PedraDB (one dir, one process)

Montanha peer process:
   network Raft / store logic  +  PedraDB on disk
```

### 3.2 Product “feel” of a write (FDB vs Montanha *today*)

| | **FoundationDB (product feel)** | **Montanha Store MVP (today)** |
|--|--------------------------------|--------------------------------|
| Client mental model | “I open a TX, read/write keys, commit” — **one** cluster TX model | “I `put` a key; the **range leader** replicates via Raft” |
| Cross-key atomicity | First-class in the core API (with size/time limits) | **Strong locally** in PedraDB; **across peers** only what the store log applies (DCS multi-key is per command / future TX work) |
| Who is “the leader”? | Hidden behind proxies / sequencers (you don’t pick a range leader by hand) | Explicit per-range leadership in the multi-Raft MVP |
| Read models | Serializable snapshot reads as part of TX | **Strong** (leader) + **fast RO** (`get_fast_replica` / LocalApplied lag metrics) — TiKV-style follower reads |

**Nuance:**  
- **Target product feel** (FDB-like): apps think in **transactions on keys**, not “Raft groups.”  
- **Current MVP plumbing** looks more TiKV-ish because ranges + Raft are visible in the API/tests.  
That is a **maturity / scaffolding** gap, not a permanent “we are TiKV.”

### 3.3 How durability across machines is built (implementation)

| | **FoundationDB** | **TiKV (for contrast)** | **Montanha (today / near)** |
|--|------------------|-------------------------|-----------------------------|
| Data plane consensus story | Unbundled: versions, resolvers, tlogs, storage | **Multi-Raft** per region | **Multi-Raft ranges** MVP (`pedradb-store`) |
| Membership / coordinators | Paxos coordinators + cluster controller | PD (+ etcd inside PD historically) | Still thin / evolving |
| Famous testing | Deterministic **simulation** at huge scale | Extensive tests; different culture | Kernel `FailingEnv` + World lab + **in-tree** lossy-net I-MAJ + seed-replay + multi-node canaries (`montanha_fdb_path`); still not FDB-scale Simulation |

**Nuance:** Saying “Montanha uses multi-Raft” answers **how we replicate this year**, not **what product we are**. FDB also shards storage; it just doesn’t market itself as “multi-Raft database.”

### 3.4 Layers and ecosystems

| | **FDB** | **Montanha** |
|--|---------|--------------|
| Layer ecosystem | Mature (Record Layer, Document Layer, big company private layers) | Early crates + docs; DCS layer is the first coordination proof |
| “Build Snowflake meta on us” | Proven pattern | Ambition / long-term, not claim |
| Language / stack | C++ core, multi-language clients | Rust monorepo, PedraDB-first |

### 3.5 Maturity (the boring difference that matters most)

| | **FDB** | **Montanha** |
|--|---------|--------------|
| Production proof | Apple, Snowflake metadata, Astra, … | Research / eng MVP |
| Ops | fdbcli, redundancy modes, years of SRE lore | Not there yet |
| Correctness bar | Battle-tested OCC + simulation | Raft majority + tests growing; **not** FDB-equivalent claims |

**Nuance:** Architectural kinship ≠ “as good as FDB.” Kinship is **shape**; maturity is **years**.

---

## 4. Where TiKV fits (so it stops stealing the story)

```text
                    PRODUCT IDENTITY
                           │
              ┌────────────┴────────────┐
              │                         │
              ▼                         ▼
     FoundationDB-shaped          etcd-shaped
     (TX core + layers)           (coord is the product)
              │
              │   we choose left
              │
              ▼
     IMPLEMENTATION TOOLKIT
              │
     ┌────────┴────────┐
     │                 │
     ▼                 ▼
 multi-Raft ranges   other (future:
 (TiKV-like tool)    FDB-like TX plane,
                     different packaging)
```

| Phrase people say | What they should mean |
|-------------------|------------------------|
| “TiKV-class power” | Can scale **writes** beyond one Raft group; range HA; majority durable | 
| “FDB-class product” | Layers; TX substrate; coord as data; not etcd identity |
| “Montanha is TiKV” | **Wrong** as product identity |
| “Montanha is FDB” | **Right as philosophy**; **wrong as “feature/ops parity”** |

TiKV itself is **closer to Montanha’s multi-Raft MVP** mechanically, and **farther** from FDB’s unbundled TX machine — but TiKV is still a **general store**, not an etcd. Montanha wants the **FDB layering religion** while currently **borrowing multi-Raft scaffolding** that TiKV also uses.

---

## 5. Concrete scenarios (intuition pump)

### Scenario: Postgres HA leader lock (Patroni-shaped)

| On FDB | On Montanha (target) |
|--------|----------------------|
| Layer or app does create/CAS on `/ha/pg1/leader` inside an FDB TX | DCS layer (or raw TX) does create/CAS on `m/pg1/leader` on the **store** |
| Same substrate as app data (different key prefix) | Same store (meta prefix); not a second etcd cluster forever |

**Same idea.** Different wire and maturity.

### Scenario: Million QPS user data

| On FDB | On Montanha (path) |
|--------|--------------------|
| Sharded storage + distributed TX machinery | Horizontal store: many ranges / leaders (multi-Raft *or* future TX plane) |
| Client still “just does TXes” | Client should *feel* “just does TXes/puts” even if plumbing is ranges |

### Scenario: “I only need a small DCS”

| On FDB | On Montanha |
|--------|-------------|
| Overkill for many teams, but works | Bootstrap single-domain Raft still exists; long-term still **layer on store**, not “Montanha = etcd” |

---

## 5.5 Measured lab limits (2026-08-13) — not field peer

**Peer trajectory:** open gaps and P0–P2 plan live in [RFC-0021](rfc/0021-montanha-fdb-tikv-parity-gaps.md).  
RFC-0017 lab completion does **not** mean FDB/TiKV parity.

**Honesty:** wall times include cold compile when first run; **test body** times below. Not Apple-scale FDB Simulation / multi-TB / geo.  

**Perf gate v0** (in-process 3-node, example `findings/perf-gate-v0-verify/perf_report.json`): put/get/PendingTx commit p50/p99 JSON — lab only, not field peer. Sim volume gate: `scripts/montanha_sim_volume_v0.sh` (8h via `PEDRA_SIM_VOLUME_WALL_SECS=28800`).

| Proof | How | Measured | Limit / claim |
|-------|-----|----------|---------------|
| 3-node TCP elect + put majority | `tcp_3node_elect_put_majority` | ~0.9s body | Localhost only; majority ≥2 of 3 |
| Multi-process DCS + index layer | `multi_process_dcs_layer_freeze` | ~1.9s body | OS process reopen; **no** dual DCS raft |
| Client NotLeader retry | `tcp_client_retry_not_leader` | ~1s body | Follower-first dial → leader put |
| ENOSPC on majority | `p21_disk_full_on_majority_blocks_commit` | ~3s body | Fail closed; heal restores put |
| Rolling restart sim | `p21_rolling_restart_majority_holds` | ~15s body | In-process partition/heal, not real process kill |
| Clock skew (logical) | `p21_clock_skew_advance_time_still_maj` | ~few s | `advance_time` ticks; not NTP skew |
| Caixote multi-VM Raft | `montanha_tcp_mesh_wire.sh raft` (m13) | lab | 3 services, mesh IPs, smoke **majority 2/3**, `/leader` role health |
| Universe A–E | `scripts/universe_abcde.sh` | ~3–4 min wall (2026-08-13) | multiprocess + fdb_path + 3× matrix + chaos; **silent_wrong=0**; E=det_io residual on Darwin |

| Capability | FDB (field) | Montanha (lab max proven) |
|------------|-------------|---------------------------|
| Multi-machine majority put | Yes | Yes — TCP localhost + caixote Linux mesh |
| Leader kill / re-elect | Continuous | In-process + TCP smoke; caixote node restart ops-manual |
| Disk full on majority | Simulation schedules | FailingEnv ENOSPC 2/3 nodes |
| Product layers on cluster | Record Layer, etc. | DCS + put_batch index pins multi-process only |
| Simulation volume | CPU-years | Seed-stable elect + lossy net I-MAJ; not Simulation-scale |

**Do not claim:** multi-TB, geo, fdbcli parity, silent_wrong=0 under full FDB fault schedules.

---

## 6. Side-by-side cheat sheet

| Dimension | FoundationDB | Montanha (intended) | Montanha (honest today) |
|-----------|--------------|---------------------|-------------------------|
| Product type | Distributed TX KV + layers | Same *shape* | PedraDB kernel + store MVP + DCS helpers |
| Core API feel | TX commit | TX + layers (target) | put/get + dcs_* + range leaders visible |
| Local engine | Storage server process | PedraDB embed | PedraDB embed |
| Replication story | Unbundled FDB stack | Implementation detail | Multi-Raft ranges in-process |
| DCS / locks | App/layer on KV | Layer on store | `dcs_create`/`dcs_cas` on store + bootstrap raft |
| Maturity | Extreme | Ambition | Early |
| Closest “don’t confuse with” | Not etcd; not SQL | Not etcd; not TiKV *identity* | Not “done FDB” |

---

## 7. Normative sentences (use these in discussions)

1. **Doctrine:** Montanha is **FoundationDB-shaped**: ordered transactional substrate + layers; coordination is data.  
2. **Kernel:** PedraDB is the **local** engine (embed); it is not the whole of FDB.  
3. **Store:** Montanha-Store is the **distributed** substrate peers share; it must become the thing layers sit on.  
4. **Plumbing:** Multi-Raft ranges are a **current tool** for majority durability and scale-out writes — **not** the brand.  
5. **Honesty:** We are **not** claiming FDB feature parity, simulation parity, or production pedigree.  
6. **Anti-goal:** Montanha is **not** “a better etcd,” and **not** “TiKV with a new name.”

---

## 8. If you were confused, you probably believed one of these

| Wrong belief | Correction |
|--------------|------------|
| “Montanha = mini TiKV” | Identity is FDB-like layers; multi-Raft is scaffolding. |
| “Montanha = FDB already” | Philosophy yes; cluster TX machine and maturity no. |
| “DCS *is* Montanha” | DCS is a **layer**; Store (+ PedraDB) is the mountain. |
| “PedraDB = distributed FDB” | PedraDB = local kernel only. |
| “FDB has no sharding” | FDB shards storage; it just doesn’t make “range leader” the client’s main idea. |
| “Multi-Raft means we rejected FDB” | No — different layer of the stack (B vs A in §1). |

---

## 9. North-star sentence (FDB-first)

**MontanhaDb climbs on PedraDB the way serious systems climb on a transactional core: FoundationDB-shaped product (KV + TX substrate, layers for DCS/SQL/HA), with whatever replication machinery (today multi-Raft ranges) is needed under the hood — coordination as data, never a second mountain named etcd, and never “TiKV clone” as the identity.**
