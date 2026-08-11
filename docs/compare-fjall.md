# PedraDB vs fjall

> fjall is the **closest active peer** in Rust: pure-safe embedded LSM KV,
> local library, not a server. If PedraDB cannot state a clear *why not just
> fjall?*, it should not exist.
>
> Sources: [fjall README](https://github.com/fjall-rs/fjall) (v3.x line, 2026),
> PedraDB positioning/architecture docs.

---

## One-line each

| | |
|--|--|
| **fjall** | Mature-ish pure-Rust **embed LSM** with a BTreeMap-like API; TX and KV-sep **optional**; general application store. |
| **PedraDB** | Tiny **database kernel**: ordered KV + **multi-key ACID as the product**; local only; built so layers / a future multi-node DB sit on top; research LSM under the hood. |

Same **job class** (local library, LSM, no multi-node).  
Different **center of gravity** (general embed kit vs minimal TX kernel + substrate).

---

## Side-by-side

| Dimension | **fjall** | **PedraDB (target)** |
|-----------|-----------|----------------------|
| **Process model** | Library, embedded | Library, embedded |
| **Multi-node / server** | **No** (“not a standalone database server”) | **No** |
| **Language / safety** | **100% safe** stable Rust | **`#![forbid(unsafe_code)]`** target |
| **Structure** | LSM (`lsm-tree` crate) | LSM (own implementation) |
| **Public API shape** | `Database` + **keyspaces** + insert/get/remove/prefix/range; optional `TxDatabase` | **`open` → `begin` → get/put/delete/range → commit** only (by design) |
| **Transactions** | **Optional** (`OptimisticTxDatabase` OCC, or `SingleWriterTxDatabase`) | **Core product** — not a mode you opt into later |
| **Default path without TX** | First-class: plain `Database` + keyspaces | **Not the pitch** — kernel is transactional |
| **Keyspaces / column families** | Yes, multiple LSM trees, cross-keyspace atomic ops | **Out of v1 surface** (layers or single ordered space) |
| **KV separation (large values)** | Optional | Target default strategy (WiscKey-class), not a user-facing mode zoo |
| **Compression** | LZ4 default (feature) | Later; not surface |
| **Compaction** | Automatic background; optional compaction filters | Lazy Leveling etc. as **implementation**, not filter API zoo |
| **Durability model** | Explicit `persist(PersistMode)`; default often OS buffers (RocksDB-like) until persist/drop | Must be clear and boring; sync policy minimal API |
| **Disk format stability** | Claims stable format; breaking → major + migration | Not there yet; must not repeat sled’s “format will break forever” |
| **Maturity** | **Shipped** (e.g. 3.1.x on crates.io), active, sponsors | **Early** (WAL slice only as of now) |
| **Stars / mindshare** | ~2k+★, real Discord/community | Greenfield |
| **Positioning** | “Embeddable KV like RocksDB in safe Rust” | “Smallest ACID ordered KV to **build on**” (FDB layer idea, local) |
| **Justifies use today?** | **Yes** — if you need embed LSM now | **Not yet** — must earn phase A (TX demo) |

---

## API surface (why fjall feels “bigger”)

**fjall mental model:**

```text
Database
  └── keyspace("items")
        insert / get / remove / prefix / range
  └── WriteBatch
  └── optional: TxDatabase / OptimisticTx / SingleWriterTx
  └── persist(mode)
  └── optional: value separation, compaction filters, metrics features
```

**PedraDB mental model (target):**

```text
Db::open
Tx::begin
  get / put / delete / range
commit / abort
```

fjall optimizes for **flexible embed** (with or without TX, multiple keyspaces).  
PedraDB optimizes for **minimum surface that still enables layers** (TX is the point).

---

## Transactions (important difference)

| | fjall | PedraDB |
|--|-------|---------|
| Base store | MVCC snapshots; plain DB can lose RMW races without TX mode | TX is the intended API |
| WriteBatch | Atomic write, **not** full interactive TX (no read-your-writes like a TX) | Interactive TX is the unit of work |
| Serializable TX | Opt-in: OCC multi-writer **or** single-writer serialized | Default path / identity |
| Isolation story | Documented: need TxDatabase for real serializable RMW | Same problem space; product is the TX API |

**Honest overlap:** if someone only needs “LSM map + optional OCC TX,” **fjall already exists**.  
PedraDB only wins if the **kernel is TX-centric**, smaller, and better as a **substrate** (including future outer DB apply path) — and eventually **faster/more correct** on that path.

---

## Technical direction

| Topic | fjall | PedraDB target |
|-------|-------|----------------|
| Implementation | Split: `fjall` + `lsm-tree` | `pedradb-core` (store + tx modules) |
| Academic LSM stack | Practical LSM; KV-sep optional | Aim WiscKey + Monkey + Lazy Leveling **as defaults when they earn benches** |
| Testing culture | CI, examples, community | Plan sim/oracle **after** justify-use (phase D), not the pitch |
| `unsafe` | 100% safe claim | forbid(unsafe) |

---

## Multi-node

**Neither has multi-node.** Both are local libraries.  
A future multi-node product might embed PedraDB (or fjall) the way TiKV embeds RocksDB — that product is **not** fjall and **not** PedraDB.

---

## When to use fjall instead of PedraDB

- You need an embed LSM **today** that is maintained and documented.  
- You want keyspaces, compression, optional TX, optional value-sep **now**.  
- You are fine with a richer API and optional transactional modes.  
- You do not need a “database kernel / FDB-layer” narrative.

**Until PedraDB has a working TX path, fjall is the rational choice for almost everyone.**

---

## When PedraDB could win (only if executed)

| Wedge | Meaning |
|-------|---------|
| **TX-first, tiny surface** | No dual personality “DB vs TxDB”; one mental model |
| **Substrate contract** | Designed as bedrock for layers + later outer multi-node DB (apply batch, local ACID) |
| **Research defaults that show up in speed/space** | Not paper names on the README — measured write amp / lookup |
| **Ruthless non-goals** | No keyspace zoo / filter zoo in v1 |

If PedraDB becomes “fjall + more options,” **it should stop** and use fjall.

---

## Risk: PedraDB is a worse fjall

| Failure mode | Avoid by |
|--------------|----------|
| Feature parity race | Keep surface smaller than fjall’s |
| Rewrite forever (sled) | Ship phase A TX kernel; freeze small API |
| “Safe LSM” only differentiator | fjall already claims 100% safe |
| Maturity gap forever | Either ship usable TX path or don’t pretend |

---

## Bottom line

```
fjall     =  best-in-class *general* pure-Rust embed LSM (TX optional)
PedraDB   =  *kernel* pure-Rust embed: ordered KV + ACID first, surface minimal
```

Same neighborhood.  
fjall is **ahead on maturity**.  
PedraDB is only justified if **power/surface** and **TX-as-identity** are real — and proven by a demo, not by architecture docs.

---

## Nuances (where the simple table lies)

### 1. “Both have transactions” — not the same product shape

| Nuance | fjall | PedraDB target |
|--------|-------|----------------|
| TX is optional | **Yes** — `Database` without TX is first-class | **No** — TX is the face of the product |
| Interactive TX | Only on `*TxDatabase` | Default |
| WriteBatch vs TX | Batch = atomic write group; **not** full TX (no read-your-writes of intermediary state) | User thinks in `Tx`, not “batch vs map” |
| Two TX modes | **SingleWriter** (one write TX at a time) **or** **Optimistic** (OCC, retries) | Need one clear story (likely OCC + docs; single-writer is a valid mode but not two products) |
| Base MVCC without TX | Snapshot reads OK; **RMW can lose updates** if you stay on plain `Database` | Don’t offer a footgun “fast path” that drops serializability by default |

**Nuance:** fjall *can* do serializable TX; many users will still open plain `Database` and think they’re fine. PedraDB’s bet is **no dual personality** — harder to misuse, smaller API, less “modes.”

### 2. Durability is easy to misunderstand

fjall (explicit):

- After insert/remove/commit, data typically hits **OS page cache**, **not** necessarily disk.
- Matches **RocksDB default** culture.
- You call `persist(PersistMode)` when you care; on `Drop`, journal tries **sync to disk**.

**Nuance:** “I committed” ≠ “survives power loss” unless you understand persist.  
PedraDB must pick a **boring default** and document it in one sentence (e.g. commit durability policy), without a zoo of modes — but must not pretend every put is fsync.

### 3. Multi-thread ≠ multi-process

fjall:

- **Multi-thread:** yes, internally synchronized; clone `Database` / keyspaces.
- **Multi-process:** **no** — single DB must not be opened from two processes.

Same for PedraDB (RocksDB-class embed).  
**Nuance:** “embedded concurrent” never means “two OS processes share one directory” unless you design that (LMDB does readers; we don’t claim it).

### 4. Keyspaces are a real product fork

fjall: each keyspace = **own physical LSM**; isolation + cross-keyspace atomic ops.

That’s power **and** surface (options per keyspace, mental model of many trees).

PedraDB v1: **one ordered key space**; namespaces = **key prefixes in a layer** (FDB style).

**Nuance:** prefix layering is enough for most “column family” needs if you have TX; physical keyspaces help operational isolation and compaction independence — trade surface vs control. PedraDB chooses **less surface**.

### 5. “Safe Rust” is table stakes, not a wedge

fjall already markets **100% safe** Rust.  
PedraDB `forbid(unsafe)` does **not** differentiate on marketing alone.

**Nuance:** safety is hygiene; win on API + correctness model + speed + substrate clarity.

### 6. Maturity asymmetry kills abstract wedges

fjall: stable-ish **3.x**, disk format stability claim, migration path on major, community, sponsors.  
PedraDB: early.

**Nuance:** every architectural advantage is **hypothetical** until phase A (TX demo). “We’ll be a better substrate” does not beat “ships today” for users.

### 7. Implementation split vs monorepo kernel

fjall = `fjall` (DB API) + `lsm-tree` (engine).  
PedraDB = modules/crates with **store vs tx** boundary, but one kernel product.

**Nuance:** fjall’s split is good engineering; PedraDB shouldn’t reinvent packaging theater — the difference is **which API is sacred** (map+optional TX vs TX kernel).

### 8. Optional features become a gravitational pull

fjall: compression, KV-sep, compaction filters, metrics features, bytes backends…

Each is reasonable. Together they pull toward **RocksDB surface area**.

**Nuance:** PedraDB’s risk is copying that gravity. Research opts should **fold into defaults** when benches prove them, not become `DatabaseBuilder::with_monkey_bloom(true)`.

### 9. Compaction filters vs layers

fjall: custom logic **during compaction** (engine hook).  
FDB/PedraDB philosophy: rich behavior in **layers using TX**, not engine plugins.

**Nuance:** compaction filters are powerful and leak engine internals into apps. PedraDB should almost always say **no** — keep the kernel dumb.

### 10. Where the wedge actually evaporates

If PedraDB ships:

- optional non-TX mode + keyspaces + filter hooks + many persist modes  

…then it **is** fjall with different branding and less maturity → **use fjall**.

If PedraDB ships:

- one TX API, clear durability, fast path, boring defaults, index layer in &lt;100 lines  

…then it’s a **different product** even if both are “Rust LSM embeds.”

### 11. Justify-use nuance (both local)

| User need | Better first answer |
|-----------|---------------------|
| Ship embed LSM this quarter | **fjall** |
| Learn / teach FDB-style layers in-process | PedraDB *if* TX API is real |
| Max ops surface / CF-like isolation | **fjall** keyspaces |
| Minimal API under your own DB product | PedraDB *if* substrate contract exists |
| Proven disk format + migration story | **fjall** today |

### 12. What we should *not* overclaim vs fjall

| Overclaim | Reality |
|-----------|---------|
| “Only we have ACID” | fjall has serializable TX modes |
| “Only we are pure safe Rust” | fjall claims 100% safe |
| “Only we are local embed” | both are |
| “We’re faster” | unproven until benches |
| “We’re the substrate for multi-node” | marketing until an outer DB exists; fjall can be embedded too |

**Real narrow claims (if earned):**

1. Smaller, TX-mandatory surface → harder to misuse.  
2. Explicit non-goals → better long-term kernel.  
3. Implementation quality (amp, recovery, sim) → trust for serious substrate.  
4. Designed apply/TX hooks for *our* future multi-node DB — still must work as a plain library first.

---

## Sources

- https://github.com/fjall-rs/fjall README (features, non-goals, transactional modes, stable disk format)
- crates.io `fjall` / `lsm-tree` (~3.1.x line)
- PedraDB: `docs/positioning.md`, `docs/architecture-refined.md`
