# Montanha layering: DCS on the store (not DCS as the mountain)

**Status:** normative architecture  
**Updated:** 2026-08-12  
**Product:** [MontanhaDb](montanhadb.md)  
**FDB vs Montanha (doctrine):** [montanha-vs-foundationdb.md](montanha-vs-foundationdb.md)  
**Normative contract:** [RFC-0013](rfc/0013-montanhadb-product.md) · [invariants ↔ tests](montanha-invariants-and-tests.md)  
**Research:** [montanhadb-deep-research.md](montanhadb-deep-research.md)

---

## 1. The sentence

**First build a correct horizontal KV (Montanha-Store on PedraDB).  
Then implement DCS / Patroni-shaped HA / locks as a thin layer on that KV —  
election and config are transactions and watches on keys, not a second database product.**

This is the FoundationDB lesson and the long-term anti-etcd move:  
etcd makes coordination **the whole product**; we make coordination **an app** on a general store.

---

## 2. Stack (bottom → top)

```text
┌─────────────────────────────────────────────────────────────┐
│  Apps / Patroni-like agents / K8s-style controllers         │
│  (use Montanha-DCS API or raw TX on meta keys)              │
├─────────────────────────────────────────────────────────────┤
│  Montanha-DCS  (layer)                                      │
│  · leader keys, members, config, leases as data             │
│  · acquire/renew = CAS / multi-key TX                       │
│  · Montanha-Live = open sessions / watches on those keys    │
│  · optional etcd-shaped façade later (Kine-like), not core  │
├─────────────────────────────────────────────────────────────┤
│  Montanha-Store  (horizontal primitive)                     │
│  · multi-Raft ranges · many writers (per range leader)      │
│  · placement / split · get/put/TX across keys               │
│  · reads: local applied vs linearizable (named)             │
├─────────────────────────────────────────────────────────────┤
│  PedraDB  (local kernel, per store/peer)                    │
│  · ordered KV + multi-key ACID · WAL/SST · one process/dir  │
└─────────────────────────────────────────────────────────────┘
```

| Layer | What it is | What it is not |
|-------|------------|----------------|
| **PedraDB** | Embed engine | Not multi-node |
| **Montanha-Store** | Distributed KV (TiKV-class) | Not “the DCS” |
| **Montanha-DCS** | Coordination **library + API** on Store | Not a separate Raft universe forever |
| **Montanha-Live** | Best-effort leadership/member streams | Not fencing truth |

---

## 3. Why DCS *on* the store (not under it)

### 3.1 If DCS is the base (etcd path)

```text
DCS product = entire CP store
     │
  one Raft group
     │
  bbolt / sqlite
```

- Election, config, *and* all CP keys share **one** write leader.  
- Scale and footguns of **etcd-class** products.  
- Data plane (if any) is a **different** system bolted on later.

### 3.2 If Store is the base (Montanha path)

```text
DCS = keys + TX + watch  ──►  Montanha-Store (multi-Raft)
App data / SQL / streams ──►  same Store (other key prefixes / ranges)
```

- **Many range leaders** → cluster-wide write throughput.  
- **Leader election for Postgres** = `Create`/`Cas` on `/ha/{cluster}/leader` — same API as any conditional put.  
- Live “who is leader” = **watch/subscribe** on that key (session hub).  
- No second consensus stack for “DCS only” once Store exists.

### 3.3 What we have *today* vs target

| Today | Target |
|-------|--------|
| `pedradb-raft` single domain + `DcsCommand` | Optional bootstrap / network path |
| **`pedradb-store` multi-Raft ranges + majority commit + strong/local reads + failover + `dcs_create`/`dcs_cas`** | **DCS on Store (MVP shipped)** |
| In-process multi-node store | Network multi-Raft + placement |

**Rule:**  
DCS is a **layer on Montanha-Store**. Bootstrap single-Raft DCS remains for network/failover demos; new coord logic should prefer **Store**.

---

## 4. How DCS maps onto Store keys

Illustrative prefixing (one coordination domain per logical cluster):

```text
/m/{cluster_id}/leader          → holder + opaque payload
/m/{cluster_id}/leader:meta     → optional; or pack rev in Store MVCC
/m/{cluster_id}/members/{node}  → member info
/m/{cluster_id}/config          → versioned config
/m/{cluster_id}/…               → failover flags, etc.
```

| DCS operation | On Store |
|---------------|----------|
| `Create` leader | TX: if key absent → put; else abort |
| `Cas` | TX: if version/rev matches → put |
| `Renew` | TX: holder + expected rev → put new payload/rev |
| `Get` local | Read from local replica of range |
| `Get` linearizable | Read via range leader / ReadIndex |
| Live stream | Watch/subscribe on prefix `/m/{cluster_id}/` |
| Multi-key layer (row+index) | **`put_batch` same range only** — cross-range → `CrossRange` hard-fail (no partial); co-locate keys under one range prefix |

**Fencing token** = Store commit version / DCS `mod_revision` exposed to agents.  
**Live hub** still best-effort; demote on failed renew remains mandatory  
([live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md)).

---

## 5. Writers: who writes what

| Who | Writes |
|-----|--------|
| App clients | User key ranges (Store leaders of those ranges) |
| HA agents | Meta keys for *their* cluster prefix (Store leader of **meta range**) |
| Placement | Region metadata (control ranges or PD-like service) |
| Not allowed | Two processes one PedraDB dir; multi-master same key without TX conflict |

So: **almost all nodes are writers** for *some* data;  
**DCS is not a special single-writer universe** except that **each meta range still has one Raft leader** (correctness).

---

## 6. Bootstrap path (honest engineering)

We do **not** wait for full TiKV to get multi-node HA for Patroni-shaped use.

```text
Phase A (now / near) — “DCS bootstrap”
  PedraDB + single-domain Raft + DcsCommand
  Ship live hub, fencing, anti-footgun defaults
  Brand: MontanhaDb Coord

Phase B — “Store”
  Multi-Raft ranges + PedraDB per peer
  Placement minimal (static then dynamic)

Phase C — “DCS on Store”
  Re-home DCS ops as TX on meta prefixes
  Live hub watches Store applies
  Optional: retire dedicated DCS-only Raft group
```

**Compatibility:** keep `DcsCommand` / agent API stable; change the **backend** from “special raft log type” to “Store TX.”

---

## 7. What we refuse

| Refuse | Why |
|--------|-----|
| DCS as the only mountain forever | Becomes etcd |
| Building full multi-Raft *only* inside DCS | Wrong product boundary |
| App data in coordination ranges | Noisy neighbor + quota hell |
| Live stream as sole fencing | Split-brain |
| Shared disk multi-writer PedraDB | Kernel invariant |

---

## 8. Relation to etcd market

From research: market wants **etcd API sometimes**, **etcd process less**.

Montanha stance:

1. **Native API:** Store TX + Montanha-DCS helpers + Live.  
2. **Optional façade:** etcd-compatible gateway **on Store** (Kine-shaped), if drop-in needed.  
3. **Never:** “MontanhaDb = etcd rewrite.”

---

## 9. Checklist (normative)

**Must:**

1. Document Store under DCS in every product pitch.  
2. Keep PedraDB free of multi-node.  
3. Design DCS keys so they **shard** cleanly (prefix per cluster).  
4. Preserve fencing rev when moving DCS onto Store.  
5. Keep Live as projection of Store/DCS commits.

**Must not:**

1. Grow bootstrap single-Raft DCS into the data plane.  
2. Require linearizable read for every “who is leader?” UI poll.  
3. Equate MontanhaDb with a DCS-only product long term.

---

## 10. North-star sentence

**MontanhaDb climbs on PedraDB: first a real store (multi-Raft, many range writers), then DCS and Patroni-shaped HA as a thin layer of transactions, watches, and live sessions on top — coordination as data, not a second mountain.**
