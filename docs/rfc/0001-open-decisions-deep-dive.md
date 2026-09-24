# RFC-0001 open decisions: deep dive (competition, nuances, regrets)

**Status:** done (research companion; decisions frozen for P0–P2; revisit only via [RFC-0012](0012-next-significant-steps.md))  
**Updated:** 2026-08-11  
**Purpose:** Ground O1–O10 in what peers actually do, where people get burned, and what PedraDB should pick for P0 vs later.

This doc is **advisory**. Normative product choices stay in RFC-0001 (amend that RFC when we lock a decision).

---

## How to read this

For each open item:

1. **What it is**  
2. **What peers do** (RocksDB, fjall, redb, Badger, LMDB, Postgres/SQLite, FDB, TiKV-shaped systems)  
3. **Nuances** people miss  
4. **Regrets / war stories** (documented or widely reported patterns)  
5. **PedraDB recommendation** (P0 vs later)  
6. **What would change our mind**

---

## O1 — Commit durability default (`fsync` vs OS buffer)

### What it is

When `commit()` returns `Ok`, how far has data gone?

| Level | Survives process kill? | Survives power loss / kernel panic? |
|-------|------------------------|--------------------------------------|
| App memory only | No | No |
| OS page cache (write, no fsync) | Usually yes | **No** |
| Disk after `fdatasync`/`fsync` | Yes | Usually yes (with working disk) |

### What peers do

| System | Default | Notes |
|--------|---------|--------|
| **RocksDB / LevelDB** | `WriteOptions.sync = **false**` | WAL to OS; process crash OK; machine crash may lose last writes. Sync writes call fsync/fdatasync. Wiki documents this explicitly. |
| **fjall** | Flush to **OS buffers**, not disk; `persist(mode)` explicit; on Drop tries sync journal | Same culture as RocksDB |
| **Badger** | `SyncWrites: **false**` in DefaultOptions | Docs: msync extra when true; default false for perf |
| **Postgres** | `fsync = on`, `synchronous_commit = on` (tunable) | Database product: prefer not lose commits; can weaken for speed |
| **SQLite** | `synchronous = FULL` typical full-safety mode; NORMAL weaker | Explicit pragma ladder OFF/NORMAL/FULL/EXTRA |
| **redb** | ACID commit via dual commit slots + careful file protocol | Designed as true ACID embed; durability is part of the product story |
| **FDB** | Cluster durability via replication + logs | Different world: “commit” means distributed durable |

### Nuances

1. **Process crash ≠ machine crash.** Non-sync WAL is still useful: kill -9 of the app often loses nothing if the kernel survived.  
2. **Group commit** (batch many TX one fsync) recovers a lot of fsync cost without lying about durability.  
3. **“I committed” UX:** users of SQLite/Postgres expect durability; users of RocksDB expect speed and often run with replica/RAID/UPS.  
4. **fsync lies:** some disks/VMs ignore barriers; EXTRA modes (SQLite) exist because of that.  
5. **Justify-use vs speed pitch:** strong default helps “correct multi-key update”; weak default helps “faster than X” marketing and matches engine peers.

### Regrets / discussions

- **RocksDB ecosystem:** countless production incidents of “we thought Put was durable” when `sync=false` + power loss. The wiki is careful; apps still get it wrong.  
- **Postgres:** turning `fsync=off` for benchmarks then forgetting in prod is a classic footgun; community strongly warns.  
- **Mobile/embed:** SQLite FULL is default culture because a single corrupted DB is catastrophic.  
- **Performance blogs** often show 10–100× gaps between sync and non-sync writes on LSM — so “safe default” can make first benches look bad vs fjall defaults.

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0** | **Durable commit by default:** WAL `fdatasync` (or equivalent) before `commit` returns Ok. Document in one sentence. |
| **P1+** | Optional `CommitOptions { sync: bool }` or `commit_relaxed()` for benches / bulk load; **never** make relaxed the only path. |
| **Also** | Consider **group commit** early if sync default is too slow under multi-thread load. |

**Would change mind:** if primary users are “cache-like” or always under a higher layer that fsyncs batches; then RocksDB-like default + loud docs.

**Status leaning:** keep RFC draft O1 = strong default for P0.

---

## O2 — Write concurrency: OCC multi-writer vs single-writer TX

### What it is

Can two threads `begin` write transactions at once?

| Model | Behavior |
|-------|----------|
| **Single-writer TX** | One write TX at a time; others wait or error |
| **OCC multi-writer** | Many write TXs; commit validates read/write sets; **abort + retry** on conflict |
| **Pessimistic locking** | Locks on keys; wait/deadlock (TiDB pessimistic, classic RDBMS) |
| **No TX** | Last write wins; lost update possible (plain RocksDB / plain fjall Database) |

### What peers do

| System | Model |
|--------|--------|
| **fjall** | Both: `SingleWriterTxDatabase` **and** `OptimisticTxDatabase` (OCC, may rerun) |
| **redb** | **Single writer**, multiple readers (MVCC); serializable by sequencing writes |
| **LMDB / heed** | **Single writer**, multiple readers (including multi-process readers) |
| **Badger** | Concurrent ACID TX with SSI-style conflict detection; retries |
| **FDB** | OCC at cluster level; client retries; no long TX |
| **TiKV/TiDB** | Optimistic (Percolator) **and** pessimistic (default since ~3.0.8) after OCC abort pain in OLTP |
| **RocksDB** | No multi-key TX; concurrent writes to memtable with other mechanisms |
| **Postgres** | MVCC + locks; complex but multi-writer |

### Nuances

1. **Single-writer is not “single-threaded reads.”** Readers can still be concurrent (redb/LMDB pattern).  
2. **OCC abort rate** explodes under hot keys / long TX — FDB and early TiDB learned this the hard way.  
3. **Pessimistic** needs lock table, deadlock detection or timeouts — big surface.  
4. **Single-writer** can be a bottleneck for multi-core write throughput but is **trivial to prove serializable**.  
5. **Outer multi-node DB** often has **one writer per Region** (Raft leader) → local single-writer TX may be **exactly** the apply model.

### Regrets / discussions

- **TiDB:** optimistic default → high abort under contention → switched **pessimistic default** (still has 2PC under the hood). Community pain: apps must handle commit errors / retries.  
- **FDB:** OCC + 5s limit; “just retry” is the culture; long interactive TX hated.  
- **redb/LMDB:** deliberately single-writer; few people call that a mistake for embed — they call multi-writer a different product class.  
- **fjall:** offering **both** modes avoids picking wrong, but **doubles API mental load** (which Db type do I open?).

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0** | **Single-writer write TX** + concurrent readers (redb-like). Ship multi-key ACID without conflict-manager complexity. |
| **P1** | Add **OCC multi-writer** if benches/users need it; one API (`begin`/`commit`) with internal policy or a single config flag — **not** two database types if we can avoid it. |
| **Avoid in core** | Full pessimistic lock manager until an outer product demands it. |

**Would change mind:** primary demo is highly concurrent multi-core writes to disjoint keys (OCC helps a lot).

**Nuance for “future TiKV”:** Raft apply is naturally single-threaded per Region → **P0 single-writer matches substrate story**.

---

## O3 — Physical keyspaces vs prefix-only namespaces

### What it is

| Approach | Mechanism |
|----------|-----------|
| **Physical keyspaces / CFs** | Separate LSM (or tree) per namespace; independent options/compaction |
| **Prefixes** | One total order; `users/` vs `idx/` is convention (FDB style) |

### What peers do

| System | Approach |
|--------|----------|
| **RocksDB** | **Column Families** — first-class; atomic WriteBatch across CFs; per-CF options; drop CF |
| **fjall** | **Keyspaces** — each own physical LSM; cross-keyspace atomic semantics |
| **FDB** | **Prefixes / directories** only — no CF in core; layers invent directories |
| **redb** | **Tables** (multiple B-trees in one file) — more like named maps than LSM CFs |
| **LMDB** | Named DBs (multiple) in one env |
| **PedraDB RFC draft** | Prefix-only v1 |

### Nuances

1. **CF/keyspace ≠ SQL schema.** It’s operational isolation (compaction, bloom, drop data).  
2. **Cross-CF atomic write** is why CFs exist instead of multiple RocksDB instances.  
3. **Prefix approach** needs careful key design; bad prefixes = bad locality.  
4. **Physical keyspaces increase surface** (create/drop, options per space, open handles).  
5. **FDB layers** prove prefixes + TX are enough for indexes, queues, directories.  
6. **Drop CF** is fast operationally; “delete all keys with prefix” is expensive without range delete support.

### Regrets / discussions

- **RocksDB:** CF explosion inside Facebook-scale apps — many CFs, hard ops; also API break when CFs were introduced (they kept backward compat carefully).  
- **TiKV:** historically careful about CF count (default CF + write/lock CFs for Percolator) — **small fixed set**, not user-defined zoo.  
- **FDB:** no CFs by design — forces good key packing; beginners invent bad layouts.  
- **fjall:** “you should probably only use a single database” — multiple keyspaces yes, but still an app discipline issue.

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0–P1** | **One ordered keyspace**; document prefix discipline (FDB subspace style). |
| **P2+** | Consider **named tables/keyspaces** only if substrate users need drop/isolation/compaction split — and keep count small. |
| **Always** | Atomic multi-key TX across prefixes (same as across CFs). |

**Would change mind:** first real embedder needs independent compaction or instant “drop tenant.”

---

## O4 — Interactive TX vs apply-batch first

### What it is

| API style | Pattern |
|-----------|---------|
| **Interactive TX** | `begin` → get → put → get → commit (app logic interleaved) |
| **Apply batch** | Build list of mutations offline → `apply(batch)` atomic (Raft state machine style) |

### What peers do

| System | Primary style |
|--------|----------------|
| **FDB** | Interactive TX (client buffers writes, reads from storage) |
| **fjall** | Both: map ops, WriteBatch, and interactive Tx databases |
| **RocksDB** | WriteBatch (atomic), no interactive multi-key TX |
| **TiKV apply** | Raft log entry → apply batch of writes to RocksDB |
| **redb** | Interactive write TX (single writer) |

### Nuances

1. **Justify-use demos** (row + index) want **interactive** or at least read-your-writes in one TX.  
2. **Outer multi-node** wants **deterministic apply(batch)** from log entries.  
3. Interactive TX can be implemented **on top of** batches (buffer writes, on commit write one batch + conflict check).  
4. **Only** shipping batch first makes “app embed” story weaker vs redb/fjall TX.  
5. Raft apply must **not** use OCC abort mid-apply — apply is already ordered.

### Regrets / discussions

- Systems that only expose WriteBatch force higher layers to reinvent read-your-writes (messy).  
- Systems that only have interactive TX make Raft apply awkward (need “blind commit” path).  
- **Best engines eventually have both**, with one implementation underneath.

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0** | **Interactive TX** as public face (justify use). Internally buffer writes → single atomic WAL+memtable publish on commit. |
| **P1/P2** | Expose **`apply_batch`** / recovery apply for substrate (no conflict check, or trust caller). |
| **Never** | Two unrelated code paths that diverge in durability semantics. |

---

## O5 — Snapshot at `begin` vs first read

### What it is

When is the read snapshot “pinned”?

| Option | Semantics |
|--------|-----------|
| **At begin** | All reads in TX see DB as of begin timestamp/seq |
| **At first read** | TX can start without snapshot; first read fixes seq (slightly more flexible) |
| **Per-statement** (SQL READ COMMITTED) | Each statement new snapshot — **not** our v1 goal |

### What peers do

| System | Typical |
|--------|---------|
| **FDB** | Read version at start (GRV); reads at that version |
| **Badger / many embed MVCC** | Timestamp at TX start |
| **Postgres RR** | Snapshot established at first read in some modes historically; RC advances |
| **fjall Tx** | Snapshot-oriented (MVCC); treat as start-bound for serializable modes |

### Nuances

1. **Begin-pinned** is easier to explain and test.  
2. **First-read** allows “begin, do CPU, then read” without holding a seq as long — minor.  
3. Holding snapshots too long blocks GC (version retention) — FDB’s 5s is extreme distributed form of this.  
4. For **local** PedraDB, long snapshots only hurt **local** GC/space — still real.

### Regrets / discussions

- SQL users confuse isolation levels when snapshot timing differs.  
- Long-lived snapshots → space bloat (need “snapshot too old” eventually — P2).  

### PedraDB recommendation

**Pin snapshot at `begin` for v1.** Document that long TXs retain versions. Add “too old” later if needed.

---

## O6 — LSM strategy on day one (simple vs Lazy Leveling)

### What it is

How aggressively do we merge levels?

| Strategy | Idea |
|----------|------|
| **Leveling (RocksDB classic)** | Size ratio between levels; heavy merge; more write amp |
| **Tiering** | Stack runs; less write amp; more read amp |
| **Lazy Leveling (Dostoevsky)** | Tier upper levels, level only largest — research optimal-ish |

### What peers do

| System | Practice |
|--------|----------|
| **RocksDB/Pebble** | Leveling defaults; huge knobs; years of compaction debt lore |
| **fjall / lsm-tree** | Practical LSM with background maintenance (not marketing Lazy Leveling) |
| **Cassandra/Scylla** | Multiple strategies (STCS, LCS, TWCS…) — ops complexity |
| **Academic** | Dostoevsky/Monkey prove better points; production lag 5–10 years |

### Nuances

1. **Wrong compaction = silent space/write death**, not just slow features.  
2. Implementing Lazy Leveling **correctly** needs cost model + sim — delays justify-use.  
3. SST **format** can leave room for multiple policies if we don’t paint into a corner.  
4. Users don’t want to choose STCS vs LCS in a “tiny kernel.”

### Regrets / discussions

- **RocksDB:** compaction storms, space amp, “why is my disk full” — endless ops literature.  
- **Scylla:** many strategies because one size doesn’t fit — but Scylla is a full DB product.  
- **Paper-first engines:** risk shipping incomplete research and stalling (sled-adjacent failure mode).

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0** | No multi-level compaction required (MemTable+WAL enough). |
| **P1** | **Simple correct** compaction (classic leveling or simple tiering) + invariant “live keys never lost.” |
| **P2** | Lazy Leveling / cost model **if** benches show win and sim covers it. |

**Do not** block P0 on Dostoevsky.

---

## O7 — Value log (WiscKey) early vs late

### What it is

Store large values in a separate append log; LSM holds keys + pointers → less write amp on compaction.

### What peers do

| System | Approach |
|--------|----------|
| **WiscKey paper** | Dramatic wins for large values |
| **Badger** | Native value log + GC (hard problem) |
| **RocksDB BlobDB / Titan** | Optional; Titan non-default complexity |
| **fjall** | Optional KV separation |
| **SurrealKV** | Value log + GC in design |
| **Classic RocksDB** | Values in LSM — painful for large values |

### Nuances

1. **GC of value log is the regret magnet** — when is a value dead? Need LSM reference or backpointers.  
2. **Small values** often **slower** with separation (pointer chase). Threshold matters.  
3. **Range scans** of large values become random reads.  
4. Coupling TX + value log crash consistency is subtle (order of WAL vs vlog).

### Regrets / discussions

- **Badger/Titan:** GC tuning, space amplification, “value log fulled” incidents.  
- **BlobDB:** optional for years because integration is hard.  
- Teams that enabled KV-sep without threshold tuning regressed small-KV workloads.

### PedraDB recommendation

| Phase | Choice |
|-------|--------|
| **P0–P1** | Values **inline** in LSM. |
| **P2** | Value log behind a **size threshold** default (e.g. only ≥4KiB or ≥1KiB), with boring GC. |
| **API** | No user-facing “enable wisckey” if possible — automatic threshold. |

---

## O8 — Max key / value size

### What peers do

| System | Limits (approx / documented) |
|--------|------------------------------|
| **FDB** | Key ≤ **10KB**, value ≤ **100KB**, TX ≤ **10MB** (hard product limits) |
| **RocksDB** | Value practical &lt; **4GB** (wiki); no tiny key limit like FDB |
| **fjall** | Key ≤ **65536** bytes; value ≤ **2^32** (from docs.rs blurb) |
| **LevelDB** | Similar large practical limits |

### Nuances

1. Hard tiny limits (FDB) force layering (split blobs) — good for distributed, annoying for embed.  
2. Unlimited values invite “store video in KV” and destroy LSM.  
3. Soft limits + errors beat silent truncation.

### Regrets

- FDB users constantly hit 100KB value limit; blob layers everywhere.  
- RocksDB users store huge values → compaction collapse.

### PedraDB recommendation

| Limit | Proposal |
|-------|----------|
| Key | Soft **64 KiB** max (error if larger); recommend &lt; 1–4 KiB |
| Value | Soft **256 MiB** max error; warn in docs that &gt; few KiB should be rare until value log |
| TX total | Soft **64 MiB** buffered writes per TX for P0 memory safety; raise later |

Numbers are **proposals** — lock in P1 with tests.

---

## O9 — Language bindings (Rust only vs C ABI)

### What peers do

| System | Bindings |
|--------|----------|
| **RocksDB** | C++ core + many language bindings |
| **fjall / redb / SurrealKV** | **Rust-first** |
| **FDB** | C API + official bindings (Python, Java, Go, …) |
| **LMDB** | C core + every language |

### Nuances / regrets

- Bindings double surface and freeze ABI early.  
- Embed Rust ecosystem doesn’t need C ABI on day one.  
- C ABI later enables Python/Go without rewrite.

### PedraDB recommendation

**Rust only until post-P1.** C ABI = separate RFC if a real consumer appears.

---

## O10 — Naming (“PedraDB” sounds like a full database)

### Nuances

- “DB” attracts expectations: server, SQL, multi-node, ops tooling.  
- Engine names (RocksDB, Pebble, fjall, redb) set better expectations.  
- Rename cost rises after crates.io publish / users.

### Recommendation

Keep **PedraDB** for now; in README lead with **“embedded library / storage kernel.”**  
Revisit rename only if confusion persists after P0 — cosmetic, not architectural.

---

## Cross-cutting regrets (apply to all O*)

| Pattern | Lesson for PedraDB |
|---------|-------------------|
| **Optional safety** | People use unsafe defaults (sync=false) in prod |
| **Two APIs for one idea** | fjall Database vs TxDatabase; TiDB opt vs pess — confusion |
| **Research before useful** | sled-like stall |
| **Ops knobs instead of good defaults** | RocksDB/Scylla complexity |
| **GC afterthought** | Badger/Titan/value-log pain |
| **Format instability** | sled alpha forever |

---

## Consolidated recommendation matrix (for RFC amendment)

| ID | P0 lock proposal | Later |
|----|------------------|--------|
| **O1** | `commit` **syncs WAL** by default | `commit` option relaxed + group commit |
| **O2** | **Single-writer** write TX + concurrent readers | OCC multi-writer |
| **O3** | **No** physical keyspaces; prefixes | Optional keyspaces if demanded |
| **O4** | **Interactive TX** public | `apply_batch` for substrate |
| **O5** | Snapshot at **begin** | “Too old” / GC policy |
| **O6** | No fancy compaction until P1 **simple** correct | Lazy Leveling if measured |
| **O7** | Inline values | Threshold value log |
| **O8** | Provisional size caps | Tune with evidence |
| **O9** | Rust only | Bindings RFC |
| **O10** | Keep name; message “library kernel” | Rename only if needed |

---

## Suggested “decide now” set (so P0.2 can start)

If we amend RFC-0001 from draft → approved with these locks:

1. **O1 strong commit**  
2. **O2 single-writer P0**  
3. **O3 prefixes only**  
4. **O4 interactive TX**  
5. **O5 begin snapshot**  
6. **O6 simple LSM later, not blocking P0**  
7. **O7 inline P0**  

Challenge these five if you disagree; the rest can wait.

---

## Sources

| Topic | Source |
|-------|--------|
| RocksDB sync default | rocksdb wiki Basic Operations (sync true/false, fsync) |
| LevelDB sync | leveldb options.h `sync = false` |
| fjall persist / multi-process | fjall README |
| Badger SyncWrites false | badger options DefaultOptions |
| Postgres fsync / synchronous_commit | postgresql docs runtime-config-wal |
| SQLite synchronous | sqlite.org pragma synchronous |
| redb single writer, COW | redb design.md |
| RocksDB CF | rocksdb wiki Column Families |
| FDB limits | FDB known-limitations (prior fetch) |
| TiDB pessimistic default | PingCAP docs (prior research) |
| Pebble vs RocksDB | cockroachdb/pebble docs/rocksdb.md |
| WiscKey / Titan / Badger | prior PedraDB research docs |

---

## Next step

Review this deep dive → pick O1–O5 (and O2 especially) → amend **RFC-0001** Locked table → start **P0.2** MemTable with those constraints.
