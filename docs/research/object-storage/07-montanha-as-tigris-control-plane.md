# Can we implement Tigris using Montanha?

**Status:** finding, 2026-08-15. Not a claim of field parity.  
**Question:** the Tigris control plane (FDB brain + n-copy block + SSD cache + async metadata push) — does Montanha occupy the FDB seat?  
**Canonical pack (caixote worktree):** `caixote/.agents/worktrees/feature-erasure-object-store/docs/research/object-storage/` — especially `06` and `07`. RFC 0181 lives there, not on `caixote` main (ends at 0180).  
**Tigris facts:** [`06-tigris-and-media-tiers.md`](06-tigris-and-media-tiers.md) · primaries [`../../references/tigris-architecture-primaries.md`](../../references/tigris-architecture-primaries.md)  
**Montanha honesty:** [`../../montanha-vs-foundationdb.md`](../../montanha-vs-foundationdb.md) · RFC-0017 / 0022 / 0023 / 0024

---

## One sentence

**Yes for the brain. No for the bytes. Do not clone thirteen FDB clusters — one Montanha SoR plus Fold is the shape we already have.**

Montanha is the slot Tigris filled with FoundationDB: ordered transactional KV that holds names, versions, pointers, small objects, secondary indexes, CAS, and the replication queue. The 5 GB video never enters that slot. Erasure coding is job 5, in the backing store, not in Montanha.

---

## Verdict

| Layer Tigris uses | Can Montanha do it? | Honest today |
|-------------------|---------------------|--------------|
| 1. S3 gateway + auth | **Not Montanha’s job** | New crate / `pedradb-http`. Stateless. |
| 2. Namespace, version, pointer, CAS, SI | **Yes — this is the product** | `Transaction` + OCC + `put_with_secondary_index` + DCS CAS. Lab, not Apple Simulation. |
| 3. Small object inline | **Yes, and the limit already matches** | Cluster `MAX_VALUE_BYTES = 100 KiB` (FDB-order). Fly publishes ~128 KiB as “instantly global.” |
| 4. SSD cache of hot bytes | **Not Montanha’s job** | Local files / page cache in front of the block store. Fold caches *metadata*, not video. |
| 5. Durable large bytes (n copies / EC) | **Must stay outside** | Pedra vlog is a *local* large-value path, not a geo object store. Raising `MAX_VALUE_BYTES` to hold videos would be the wrong product. |
| 6. Place and move (push meta, pull bytes) | **Yes, but not as 13 FDB clusters** | RFC-0024 **Fold** already pushes a prefix to leaves. QuiCK-on-N-clusters is P2 and we do not need it. |

Philosophy match is exact: [foundationdb-layers-and-products.md](../../foundationdb-layers-and-products.md) already lists Tigris as “private layer on FDB.” Montanha’s doctrine is that layer. Maturity is not.

---

## What Tigris asks of the KV, mapped to code

Tigris’s own architecture page: metadata is transactional so they can offer CAS, multi-object TX, and query — “none of which is provided by S3.” Bytes go to a **block store**. The distributor is a **persistent queue inside FDB** (QuiCK).

| FDB job | Montanha today | Gap |
|---------|----------------|-----|
| Ordered keyspace | Pedra + ranges | — |
| Multi-key TX, snapshot reads, OCC | RFC-0023 `Transaction`, `get_at_version`, Conflict / TooOld | Not FDB SSI + global sequencer. Good enough for a metadata layer. |
| Cross-key atomic “row + index + enqueue” | `put_with_secondary_index`; recipes `IndexedUsers` + `Queue` in one `Transaction` | Same-TX index is **SHIP** (L31). |
| CAS | DCS create/cas; TX read-set | — |
| Directory / names → integers | `fdb_layers::{Naive,Safe}Allocator` + `Subspace` pack | Toy, not FDB Directory Layer. P0 of an object store can use raw packed prefixes. |
| Queue = range scan by time | `montanha-fdb-recipes::Queue` (head/tail RMW) | **Not** `SET_VERSIONSTAMPED_KEY`. Enqueues serialize on the tail key. Fine for a lab / one-region queue; not Tigris-scale QuiCK. |
| Watch after commit | `WatchHub` post-majority | Not etcd gRPC. Enough to wake a replicator. |
| Multi-cluster metadata replica | **No** | RFC-0021 P2.6 is **region-prefer dial**, not async DR. |
| Regional metadata cache | **Fold** (RFC-0024) | Fold is LocalApplied, not linearizable — which is the *same* consistency Tigris’s default push gives you. |
| Snapshot / fork of a bucket | **Layer, not kernel** | Ordered keys + inverted `u64` stamp is a recipe. `get_at_version` is a different (MVCC) snapshot. Tigris’s trick does not need a new primitive. |
| Atomic ADD / conflict-free counters | **REFUSE** (L34) | Not required for object metadata. |
| 5 s TX / 10 MB / 100 KB | Cluster: 100 KiB / 10 MB / 10 k keys. **No** 5 s wall clock (L30 REFUSE) | Keep the 100 KiB cap on the *cluster* path. Do not lift it “to store objects.” |

`Queue::push` today:

```359:369:crates/montanha-fdb-recipes/src/lib.rs
    pub fn push(&self, cluster: &mut StoreCluster, value: &[u8]) -> Result<u64> {
        let mut tr = cluster.begin();
        let tail = Self::parse_u64(tr.get(cluster, self.tail_key())?);
        // ... set item + tail+1 ...
        tr.commit(cluster)
    }
```

That is a correct FDB *recipe* queue, not a versionstamp queue. A Tigris-shaped replicator that enqueues every PUT through one tail will OCC-conflict with itself. P1 of the layer is either (a) a real versionstamp fill at commit, or (b) many partitioned queues (QuiCK’s actual trick).

---

## The topology we should build (if we build family B)

Do **not** stand up 13 Montanha clusters and write a QuiCK clone. We do not have 13 regions. Tigris did that because each region is its own FDB and they refuse a WAN commit.

What we already shipped is closer to CloudKit-on-one-cluster than to Tigris-on-thirteen:

```
S3 gateway (stateless, any cell)
    │
    ├─ local SSD / page cache     → hot bytes
    ├─ Fold (RFC-0024)            → metadata LocalApplied
    │                               “Chicago already knows the object exists”
    ├─ Montanha (one SoR)         → TX: record + pointer + enqueue
    │                               inline if len ≤ 100 KiB
    └─ block / object backing     → n copies
         · local NVMe (caixote 0091 / host volume)
         · B2 / S3 / Garage (0058 / 0144)
```

PUT:

1. If `len ≤ 100 KiB`: one Montanha TX writes the bytes **in the record**.
2. Else: write bytes to NVMe (and/or B2) **first**, then one Montanha TX writes `{etag, size, regions: [here], ptr}`. Orphan bytes on TX abort are GC, same as every object store.
3. Fold tails the metadata prefix. Other cells see the name in seconds without a second database.

GET in another cell:

1. Fold hit → pointer → pull bytes from origin NVMe/B2 → fill local cache.
2. Fold miss / want linearizable → `get_strong` on Montanha (Tigris “global strong consistency” = proxy to the leader).

A cell that dies still serves the photo **if** the Fold already has the pointer **and** the backing store is a different failure domain (B2 / another host). That is the meteor sentence, without thirteen brains.

This is family **B**. Job 5 is “someone else’s object store + our NVMe,” not Reed-Solomon on our HDD.

---

## What we must not do

1. **Put the video in Montanha.** Cluster path rejects `> 100 KiB`. That reject is correct. Pedra vlog is for the *local* engine, not for S3.
2. **Build EC in P0 because “object storage = EC.”** EC is how *family A* uses HDD. Tigris did not document it; we should not either until we compete on $/GB.
3. **Call Fold linearizable.** It is not. Neither is Tigris’s default metadata push.
4. **Claim FDB field peer.** RFC-0021 residual: no production pedigree, no geo-HA, no Apple Simulation, TLS still lab-default cleartext.
5. **Become Tigris.** Different company, different 13-region bet. Steal the six-job split and the inline+pointer schema. Leave anycast + QuiCK-across-clusters for a day we have the regions.

---

## If we ever ship this: slices (layer, not kernel)

### P0 — useful in one region

- S3 subset (Put/Get/Head/Delete/List) on a stateless gateway.
- Montanha record: `(tenant, bucket, key) → {etag, size, regions, ptr | inline}`.
- Inline iff `len ≤ MAX_VALUE_BYTES`.
- Bytes otherwise: local NVMe file or B2 object; pointer committed in the same logical PUT (bytes first).
- CAS via `If-Match` / TX read-set (Tigris/S3 conditional write).
- Tests: crash between byte write and meta TX leaves no visible object; retry is idempotent; `> 100 KiB` never lands in the KV.

### P1 — the features that are *why* they used FDB

- Secondary index (prefix list, etag) in the **same** TX (already a recipe).
- Bucket snapshot / fork as inverted-`u64` WAL per key (Tigris blog; ordered keys only).
- Replicator woken by `WatchHub`, not a second Kafka.
- Versionstamp **or** partitioned queues before the enqueue key becomes the bottleneck.

### P2 — only with real extra regions

- Fold of the metadata prefix in every cell (already the fold product).
- Residency header: bytes do not copy; GET is proxy.
- Eager byte push (“Accelerate”) as a bucket flag.
- Multi-Montanha QuiCK **only** if we ever run independent SoRs. Default: do not.

---

## Family A vs family B for Armazém

| | Family A (MinIO/Ceph/B2) | Family B (Tigris / us-if-we-steal) |
|--|--------------------------|-------------------------------------|
| Montanha’s job | Job 2 only (namespace, heal map) | Jobs 2, 3, 6 |
| Bytes | Our HDD + Reed-Solomon | Our NVMe + their B2/S3 |
| Cell dies | Reconstruct from *k* shards | Pointer already elsewhere; pull |
| Compete on | $/GB | Small-object latency + one global bucket |
| P0 | Hard (disks, NIC, heal) | Gateway + TX layer + a bucket we already provision |

Caixote today **is already B for the customer** (RFC 0058 B2, 0144 BYOBucket). An Armazém that rebuilds family A from zero is a different company than an Armazém that puts Montanha in the FDB seat and keeps B2 under the pointer.

RFC 0091 (elastic disk) is the *block* ladder (NVMe → peer → S3). Family B object storage is the same ladder with an S3 API and Montanha as the index. Do not merge the two products; share the backing stores.

---

## Texto para RFC 0181 (addendum, quando o RFC existir)

> **Family B / Tigris: o que não somos.**
>
> Object storage tem seis empregos. Erasure coding é o 5. Tigris (docs + Fly) não é um cluster EC: FoundationDB guarda nome, versão, ponteiro e objectos ≲ 100–128 KiB; bytes grandes vão para NVMe + object store fora da rede; metadado é push assíncrono; bytes são pull. Snapshot/fork é WAL de metadado em KV ordenado, não cópia de terabytes.
>
> Armazém P0 **não** clona Tigris e **não** trata EC como o store inteiro.
> - Metadado fora do disco EC (já no RFC).
> - Inline no Montanha abaixo de `MAX_VALUE_BYTES` (100 KiB). Não subir esse tecto para caber vídeo.
> - FAST = n cópias em NVMe; STANDARD = backing store (B2/S3 ou, mais tarde, HDD+EC).
> - Push global de metadado **não** é P0. Um Montanha + Fold (RFC-0024) cobre a janela “Chicago já sabe que existe.”
> - EC, se algum dia existir, está **debaixo** do backing store. Não no produto que documentamos no dia um.

Caixote `rfcs/` ends at **0180** as of this note. Do not invent 0181 here.

---

## Sources checked this session

| Claim | Primary | Verdict |
|-------|---------|---------|
| FDB = metadata; bytes = block store | tigrisdata.com/docs/concepts/architecture | confirmed |
| Queue = QuiCK on FDB | same page | confirmed |
| Multi-cluster FDB + their replicator | same page | confirmed |
| Inline small objects; Redis-like | fly.io/customers/tigris + small-object blog | confirmed |
| ≲ 128 KiB instantly global | fly.io/blog/tigris-public-beta | confirmed; **not** 1 MiB |
| NVMe Fly + off-network object store | Fly customer story + community staff post | confirmed |
| Snapshot = inverted u64; fork = recurse parent | tigrisdata.com/blog/bucket-forking-deep-dive | confirmed (code snippet in post) |
| Tigris publishes Reed-Solomon | architecture + blogs fetched | **not found** — do not claim they do or don’t beyond “unpublished” |
| Montanha 100 KiB / TX / SI / Queue / Fold | in-tree RFCs 0022–0024 + `MAX_VALUE_BYTES` | confirmed in code |
