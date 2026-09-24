# DCS / coordination market: is etcd “the best”?

**Status:** research note (facts + market role)  
**Updated:** 2026-08-12  
**Sources:** etcd docs [versus other stores](https://etcd.io/docs/v3.5/learning/why/), Patroni docs, k3s datastore docs, Kine/PG blogs, Clever Cloud FDB-backed etcd API (2025), TiDB PD wiki.  
**Product design using this research:** [live-leadership-and-patroni-shaped-ha.md](live-leadership-and-patroni-shaped-ha.md)

---

## Short answer

**No.** etcd is the **default / incumbent for Kubernetes-shaped control planes** and a solid **CP metadata KV**, not the universal “best DCS on Earth.”

The market is **fragmented by job**:

| Job | What wins in practice |
|-----|------------------------|
| K8s apiserver state | **etcd** (default); alternatives via **Kine** (SQLite/PG/MySQL) or experimental FDB layers |
| Postgres HA (Patroni) | **etcd *or* Consul *or* ZK *or* Kubernetes API** — multi-backend by design |
| Service discovery + multi-DC + health | **Consul** (or mesh products), not etcd |
| Hadoop/Kafka-era coordination | **ZooKeeper** (legacy bulk, still huge installs) |
| China mid-platform config/discovery | **Nacos** (often preferred locally over etcd) |
| “Coordination as data” on a real DB | **FDB / Spanner / Cockroach / TiKV+PD** patterns — election = TX, not a separate product |

---

## What each product actually is

### etcd
- **Role:** distributed `/etc` — small CP keyspace, one Raft group, bbolt, watches, leases, txn/CAS.
- **Wins because:** K8s default; CNCF gravity; simple mental model for “one consistent log of cluster state.”
- **Loses as “best DCS” because:** single raft group doesn’t scale horizontally; fsync/disk sensitive; quota/compact footguns; not multi-DC native; not service discovery end-to-end.
- etcd’s **own** docs admit bias and position vs Consul as **different problems** (KV consistency vs full service discovery stack).

### ZooKeeper
- Same problem class (coordination metadata); older, Java/Zab, recipes often external (Curator).
- Still huge in **Kafka/Hadoop/legacy** estates.
- etcd markets itself as “ZK with hindsight”; market reality = **inertia**, not “ZK is better.”

### Consul
- **Service mesh / discovery first**; KV is secondary.
- Strong multi-DC story relative to etcd federation.
- Patroni supports it as DCS; ops teams that already run Consul often **skip etcd entirely** for PG HA.

### Nacos
- Alibaba ecosystem: config + discovery, big in **CN cloud**.
- Not “better CP theory” than etcd — **better fit** for that market’s stack.

### Kine + SQLite / PostgreSQL / MySQL
- k3s and others: **etcd API façade** over SQL.
- Proves the market wants **etcd’s *API contract for apiserver***, not necessarily etcd the process.
- SQLite = single-server only; PG/MySQL = HA you already know how to run.

### FoundationDB / NewSQL as “DCS substrate”
- Clever Cloud (2025): managed K8s with **etcd-compatible API on Materia KV (FDB)** — explicit “we rethought etcd.”
- Google historically: **Spanner** for large control planes, not etcd-at-planet-scale.
- etcd docs themselves: NewSQL for **TB-scale / SQL**; etcd for **few GB metadata** — i.e. they are different design points, not etcd winning everywhere.

### TiDB PD
- PD **embeds etcd** for its own metadata — even “modern NewSQL” still uses etcd-shaped raft for the **tiny** control kernel. That is endorsement of the **pattern** (small consistent store), not of etcd as the only product forever.

---

## “Best DCS” depends on the question

| Question | Honest answer |
|----------|----------------|
| Best **default** for vanilla Kubernetes? | **etcd** (ecosystem, not pure merit) |
| Best **ops experience** if you already run Consul? | Often **Consul** for Patroni/discovery |
| Best **embed / edge / CI k8s**? | **SQLite via Kine** (k3s) |
| Best **reuse existing HA Postgres**? | **Kine + PG** |
| Best **scale-out metadata** with TX as election? | **FDB / Spanner-class**, not etcd |
| Best **pure coordination recipes library** historically? | ZK + Curator (legacy) |
| Best **API shape for K8s without running etcd**? | **etcd API** (Kine / FDB layer) — note: API ≠ etcd binary |

---

## Implication for PedraDB

Copying **“be etcd”** is a weak strategy: market already has etcd + Kine + Consul + Nacos + cloud control planes.

Stronger strategies aligned with market pressure:

1. **Local ACID ordered KV** (PedraDB kernel) — embed, not “another etcd.”  
2. **Horizontal KV + TX** — election/lock as **transactions** (FDB lesson; Clever Cloud path).  
3. **Optional etcd-compatible façade** only if you need K8s/Patroni drop-in — like Kine, not like forking etcd’s ops model.  
4. **DCS doctrine without etcd footguns** — see [multi-node-without-etcd-footguns.md](multi-node-without-etcd-footguns.md).

**Patroni already proves etcd is not unique:** multi-DCS is a first-class product requirement.

---

## Bottom line

- **Market leader for K8s CP storage:** etcd (incumbent).  
- **Best DCS in absolute terms:** **does not exist** — job-specific.  
- **Trend:** keep **etcd API** where clients demand it; replace **etcd process** with SQLite/PG/FDB/distributed SQL when ops or scale hurt.  
- Building “a better etcd clone” is a crowded, footgun-rich niche. Building a **better coordination substrate** (TX + ordered KV, multi-writer by range) is where the interesting market is moving.
