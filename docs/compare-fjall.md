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

## Sources

- https://github.com/fjall-rs/fjall README (features, non-goals, transactional modes, stable disk format)
- crates.io `fjall` / `lsm-tree` (~3.1.x line)
- PedraDB: `docs/positioning.md`, `docs/architecture-refined.md`
