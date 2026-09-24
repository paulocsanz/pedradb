# Live leadership + Patroni-shaped HA (design detail)

**Status:** normative product design (living)  
**Updated:** 2026-08-12  
**Product:** [MontanhaDb (Montan-HA-DB)](montanhadb.md) — this doc is the **leadership / Patroni-shaped HA** design under that brand  
**Audience:** product + engineering (PedraDB kernel + Montanha coordination layers)  
**Related docs:**

| Doc | Role |
|-----|------|
| [pedradb-as-dcs-storage-for-patroni.md](pedradb-as-dcs-storage-for-patroni.md) | PedraDB = local engine under a DCS product |
| [multi-node-without-etcd-footguns.md](multi-node-without-etcd-footguns.md) | Multi-nó CP sem footguns clássicas do etcd |
| [dcs-market-landscape.md](dcs-market-landscape.md) | Mercado: etcd não é o único/melhor DCS |
| [apply-and-raft.md](apply-and-raft.md) | Apply log + Raft + `DcsCommand` |
| [doctrine-primitives-and-api-layers.md](doctrine-primitives-and-api-layers.md) | Kernel vs camadas de produto |
| [usage.md](usage.md) | API surface shipped |

---

## 1. Problem statement

We want a **Postgres HA manager model in the spirit of Patroni**:

- Distributed agreement on **who is primary**
- Members, config, failovers coordinated via a **DCS**
- Clients and proxies should, **whenever possible**, keep a **long-lived connection** and see **live** updates of who is / is not leader

We do **not** want:

- Silent split-brain (two primaries both accepting writes)
- etcd-style footguns as the product identity (linearizable-everything, unbounded MVCC, NOSPACE freeze, poll storms)
- Treating a **best-effort live stream** as the source of truth for fencing

**One-sentence goal:**  
**Strong election on the DCS; best-effort live view over open sessions; fencing always by revision/token.**

---

## 2. Background: what Patroni actually needs

Patroni (and similar HA managers) do **not** need a general database. They need a **Distributed Configuration Store (DCS)** for:

| Concern | Semantics | Typical etcd usage |
|---------|-----------|-------------------|
| Leader lock | At most one holder; race-free acquire | Create / CAS + lease |
| Leader renew | Holder proves liveness | Keepalive / update |
| Members | Per-node registration + heartbeat | Keys under prefix |
| Cluster config | Versioned cluster settings | Conditional put |
| Failover / sync state | Coordinated switchover flags | Extra keys |
| Watch / react | Fast reaction to leader change | Watch API |
| Initialize | Only one bootstrap | Create-if-absent marker |

Patroni already supports **multiple DCS backends** (etcd, Consul, ZooKeeper, Kubernetes). That is market proof that **etcd is a backend, not the definition of HA**.

PedraDB’s role in this stack:

```text
HA manager / agents / proxies / apps
        │
        ▼
  DCS product (pedra-dcs + optional Raft + live hub)
        │  in-process / network
        ▼
  PedraDB (local ordered KV + multi-key ACID)
```

- **PedraDB** = local durable state machine storage (bbolt’s role inside etcd).  
- **DCS product** = coordination semantics + multi-node agreement + live sessions.  
- **Not** “PedraDB speaks etcd wire by default.”

---

## 3. Market context (why not “just be etcd”)

Detailed note: [dcs-market-landscape.md](dcs-market-landscape.md).

Summary facts:

| Job | What the market uses |
|-----|----------------------|
| Vanilla Kubernetes control plane | **etcd** (incumbent default) |
| etcd API without etcd process | **Kine** → SQLite / PostgreSQL / MySQL |
| etcd-compatible on horizontal KV | e.g. FDB-backed layers (industry experiments / managed offerings) |
| Postgres HA (Patroni) | **Multi-backend**: etcd, Consul, ZK, K8s |
| Service discovery + multi-DC | **Consul** (and mesh products) more than etcd |
| China config/discovery platforms | **Nacos** often preferred |
| Tiny control kernel inside NewSQL | e.g. TiDB PD still uses **etcd-shaped** embedded raft for metadata |

**Conclusion for Pedra:** copying “full etcd product” is a weak strategy. Better:

1. Local ACID ordered KV (kernel).  
2. Coordination as **data + TX / Raft apply** (DCS layer).  
3. Optional etcd-compatible façade later if drop-in is required (Kine-style).  
4. Long-lived **leadership sessions** for live UX/ops without linearizable-get spam.

---

## 4. Core architecture: two planes

### 4.1 Plane A — Truth (strong)

**Purpose:** decide and fence who may be primary.

| Property | Rule |
|----------|------|
| Mechanism | `create` / `cas` / renew on leader key via DCS (`DcsCommand` on Raft when multi-node) |
| Writer | Single Raft leader proposes; followers apply same log |
| Identity of leadership | Holder of key + **fencing token** (`mod_revision` / cluster revision) |
| Durability | Majority Raft commit + PedraDB WAL policy on apply |
| Failure mode | `NotLeader`, `CasFailed`, quorum loss — typed, not silent |

**Invariant:** at most one holder of a given leader key in the committed history (for a given fencing regime).  
If two agents believe they are primary, **at most one** has the current `rev`; the other must demote on renew/check.

### 4.2 Plane B — Live view (best-effort)

**Purpose:** keep clients informed **quickly** of leadership and membership hints.

| Property | Rule |
|----------|------|
| Mechanism | Long-lived session (stream / long-poll / gRPC bidi) |
| Content | Snapshots + deltas (`LeaderChanged`, member hints, …) |
| Consistency | **May lag**; may briefly show a stale leader after partition healing |
| Must not be used alone for | Critical write fencing |
| Reconnect | Cursor (`since_revision`); never “from now” by default |

```text
┌──────────────────────────────────────────────────────────┐
│  Apps · proxies · UI · orchestrators                     │
│  SubscribeLeadership(cluster)  ──best-effort live──►     │
└───────────────────────────▲──────────────────────────────┘
                            │ push on change
┌───────────────────────────┴──────────────────────────────┐
│  Leadership session hub (per coordination domain)        │
│  · open connections when possible                        │
│  · fan-out events                                        │
│  · cursor / resync                                       │
└───────────────────────────▲──────────────────────────────┘
                            │ after commit / apply
┌───────────────────────────┴──────────────────────────────┐
│  Plane A: DCS + Raft                                     │
│  DcsCommand create/cas/put/delete · revision · members   │
│  PedraDB local per node                                  │
└──────────────────────────────────────────────────────────┘
```

**Rule of composition:**  
Plane B is a **projection** of Plane A (plus optional agent heartbeats).  
Plane A never depends on Plane B for correctness.

---

## 5. Consistency levels (explicit, never silent)

Every read/subscribe API **names** its guarantee:

| API (conceptual) | Guarantee | Typical use |
|------------------|-----------|-------------|
| `SubscribeLeadership` | Best-effort, ordered events per session, may lag | UI, proxy reconfig, ops |
| `GetLeader(local)` | Applied state on this node (may lag commit) | Cheap “who do we think?” |
| `GetLeader(linearizable)` | Leader/ReadIndex-class; rare | Audit, rare critical path |
| `AcquireLeader` / `RenewLeader` | Strong CAS/create + fencing rev | Agent promote/demote |
| `WriteAsPrimary(rev)` | App/agent checks `rev` before accepting writes | Fencing |

**Anti-etcd rule:** do **not** make linearizable Get the default for every status call.

---

## 6. Leadership session protocol (detailed)

### 6.1 Session lifecycle

1. **Open**  
   - Client connects to hub (TCP framed / HTTP long-poll / gRPC stream — wire TBD).  
   - Authenticates (future); binds to `cluster_id` / coordination domain.  
   - Sends `Subscribe { cluster_id, since_revision?: u64 }`.

2. **Snapshot**  
   - Hub sends immediate `Snapshot` with:
     - `leader_holder` (bytes / node id)
     - `leader_rev` (fencing token)
     - `members[]` (best-effort registry)
     - `hub_time` / generation  
   - If `since_revision` is set and still in retained history, hub may send **only deltas** after a compact snapshot header.

3. **Steady state**  
   - Connection stays open.  
   - App-level heartbeat (e.g. every 10–30s) optional for liveness of the **session**, not of Postgres primary.  
   - Hub pushes events as they occur (see §6.3).

4. **Reconnect**  
   - Client reconnects with `since_revision = last_seen_rev`.  
   - If hub compacted past that → `CursorGone` + full snapshot.  
   - **Never** default to “subscribe from now” (silent gap).

5. **Close**  
   - Client closes; hub drops fan-out entry.  
   - No effect on Plane A leadership.

### 6.2 “Always open when possible”

| Situation | Behavior |
|-----------|----------|
| Network healthy | Keep one long-lived stream per client interest |
| Transient blip | Client auto-reconnect with cursor; exponential backoff + jitter |
| Hub restart | Clients reconnect; snapshot from DCS applied state |
| Client mobile / flaky | Allow long-poll fallback; same event model |
| Partition: client with minority | Stream may show stale leader; client must not fence from stream alone |

### 6.3 Event types

All events carry at least: `cluster_id`, `event_rev` (DCS revision or hub sequence), `ts`.

| Event | Meaning | Strong? |
|-------|---------|---------|
| `Snapshot` | Full current view | Best-effort view of applied state |
| `LeaderChanged { holder, rev, prev_holder? }` | Committed change to leader key | Truth was committed; delivery is best-effort |
| `LeaderCleared { rev }` | Leader key deleted / expired | Same |
| `MemberUpsert { node, meta }` | Member key updated | Registry (weaker if TTL local) |
| `MemberGone { node, reason }` | Heartbeat miss or explicit leave | **Suspicion**, not proof of death |
| `RoleHint { node, role }` | What agent claims (primary/replica) | Hint only |
| `LeaseWarning { key, ttl_remaining }` | Approaching expiry | Best-effort ops |
| `CursorGone { min_rev }` | Must full resync | Strong protocol signal |
| `HubBye { reason }` | Graceful hub shutdown | — |

**Who is / is not leader (live):**

- **Is leader (official, if stream caught up):** `holder` in last `LeaderChanged` / `Snapshot` with max known `rev`.  
- **Is not leader:** any `node != holder` for that key.  
- **Unknown:** disconnected, or `CursorGone` until resync.

### 6.4 Ordering guarantees (session)

For a single session stream:

1. Events are delivered in **non-decreasing `event_rev`** order.  
2. After reconnect with cursor, no event with `rev <= since` is re-delivered unless snapshot restates it.  
3. Hub may coalesce multiple rapid changes into one `LeaderChanged` **only if** intermediate revs are not required by the client contract; default is **no silent skip** of leader holders (last writer wins is OK only if documented as “latest only” mode).

**Default mode:** deliver every committed leader change (or a dense log).  
**Optional mode `latest_only`:** coalesce for UI; not for proxies that need every transition.

---

## 7. Plane A: election and fencing (Patroni-shaped)

### 7.1 Leader key

Example layout (illustrative; exact encoding is product choice):

```text
d/k/cluster/{id}/leader     → holder identity (node id, conninfo, …)
d/m/cluster/{id}/leader     → meta: create_rev, mod_rev, lease
d/k/cluster/{id}/members/{node} → member payload
d/rev                       → global DCS revision
```

### 7.2 Acquire (race)

```text
DcsCommand::Create { key: leader_key, value: holder, lease: 0|L }
  or Cas { expected_rev: 0, ... }
```

- Multi-node: only Raft **leader** proposes; `check_command` then log; all nodes `apply_dcs_command`.  
- Exactly one Create-if-absent wins in the log order.  
- Losers get `CasFailed` / network error with reason.

### 7.3 Renew

```text
RenewLeader(key, holder, expected_rev, lease?) → new_rev | LostLeadership
```

Semantics:

1. Read current key.  
2. If missing or `holder` mismatch or `mod_rev != expected_rev` → **LostLeadership** (demote).  
3. Else CAS to same holder with updated payload / lease → `new_rev`.  
4. Agent stores `new_rev` as fencing token.

### 7.4 Fencing token (mandatory for safety)

| Actor | Rule |
|-------|------|
| Primary agent | Before accepting writes / after promote: confirm `rev` still mine via renew or linearizable check |
| Stale primary | If renew fails or stream shows higher `rev` with other holder → **demote immediately** |
| Proxies | Prefer stream for **routing**; still ready for connection errors on old primary |
| Apps | Ideally never decide primary themselves; use proxy or always retry |

**Stream speeds demotion; renew is authoritative.**

### 7.5 Leases / TTL (when used)

Doctrine (anti-footgun):

- **Min TTL floor** and **max TTL ceiling** for coordination keys.  
- Expiry deletes or tombstones leader key only through logged commands (multi-node).  
- Prefer **explicit fencing** over “clocks agreed TTL” alone.  
- Document: lease expiry on server vs client clock skew → client must not assume ownership without successful renew.

(Current multi-node path often uses `lease: 0` first; leases as replicated first-class are a follow-on.)

---

## 8. Agents, proxies, and open connections

### 8.1 HA agent (Patroni-like process next to Postgres)

Responsibilities:

1. Participate in acquire/renew (Plane A).  
2. Promote/demote Postgres.  
3. Optionally publish `RoleHint` / member heartbeats.  
4. Subscribe to leadership stream (Plane B) for **fast** demote on `LeaderChanged`.  
5. Keep **its own** management connection to hub/DCS open when possible.

### 8.2 Proxy / connection router

- Maintains long-lived **SubscribeLeadership**.  
- On `LeaderChanged`, rewrites backend pool (or labels).  
- Client Postgres connections may be:
  - **drained** on failover, or  
  - left to fail and retry (simpler).  
- Proxy must tolerate **stale stream** (briefly routing to old leader → errors → retry).

### 8.3 Application connections (Postgres)

Two products, do not confuse:

| Connection | Meaning |
|------------|---------|
| App → Postgres | Query path; primary vs replica is **routing**, not DCS |
| App/agent → Leadership hub | Control path; live who-is-leader |

“Manter conexão aberta” no design **principalmente** refere-se ao **hub de leadership** (e opcionalmente pool do proxy).  
Manter conexões Postgres sticky ao primary através de failover is optional and hard; default is fail+reconnect.

---

## 9. Multi-node topology

### 9.1 Process boundaries

```text
Node 1                         Node 2                         Node 3
┌─────────────────────┐       ┌─────────────────────┐       ┌─────────────────────┐
│ pedra-raft-node     │◄─────►│ pedra-raft-node     │◄─────►│ pedra-raft-node     │
│  Raft + DCS apply   │  TCP  │  Raft + DCS apply   │       │  Raft + DCS apply   │
│  PedraDB data dir   │       │  PedraDB data dir   │       │  PedraDB data dir   │
│  optional hub shard │       │  optional hub       │       │  optional hub       │
└─────────────────────┘       └─────────────────────┘       └─────────────────────┘
```

- **No shared data directory** across processes.  
- Exclusive `LOCK` per PedraDB dir.  
- Raft log + hard state persisted (`RAFT_HARD` / `RAFT_LOG`).

### 9.2 Who accepts DCS writes?

- Only the **Raft leader** accepts `propose_dcs` / mutations.  
- Followers serve `dcs_get` (applied) and can run hub fan-out from local applied state **or** forward subscribe to leader (implementation choice).

**Recommended hub placement (first ship):**

- Hub runs **co-located on every node**, feeding from **local apply hook** after `apply_dcs_command`.  
- Clients may connect to any node; if node is partitioned from quorum, stream may be stale (documented).  
- Optional: clients prefer connecting to Raft leader for lower lag (hint via status RPC).

### 9.3 Coordination domain

- One Raft group = one **coordination domain** (e.g. one Patroni cluster, or one product environment).  
- Do not put application data tables in the DCS raft group.  
- Hard max value size for DCS keys (product limit).

---

## 10. Anti-footgun rules (etcd + classic Patroni)

Aligned with [multi-node-without-etcd-footguns.md](multi-node-without-etcd-footguns.md):

| Footgun | Rule in this design |
|---------|---------------------|
| Linearizable read default | Live stream + local get default; linearizable opt-in |
| Poll storms on DCS | Open session + push on commit |
| Lost watch events | Cursor + `CursorGone` |
| Unbounded revision history | Short retention for DCS history; auto compact policy |
| NOSPACE brick | Soft quota + reclaim path; typed full error |
| Lease silent death | Events + renew authority + fencing rev |
| “Stream said I’m not leader so I’ll race CAS wrong” | Stream only demotes; acquire still CAS |
| Even-sized clusters | Reject even voter counts |
| Two clusters merge | Cluster ID on every RPC |
| Shared NFS for raft data | Forbidden in ops doctrine |

---

## 11. Failure scenarios (worked)

### 11.1 Clean failover

1. Primary agent fails renew or operator switchover.  
2. Leader key CAS to new holder (or Create after delete).  
3. Raft commits `DcsCommand`.  
4. All nodes apply; hubs push `LeaderChanged`.  
5. Old primary: renew fails and/or stream event → demote.  
6. Proxies move traffic.  
7. Clients with open Postgres conns error and reconnect.

### 11.2 Network partition (primary island)

1. Primary loses quorum; cannot commit renew.  
2. Majority elects new Raft leader; new holder acquires key.  
3. Island primary still **thinks** it is primary until renew fails or local timeout.  
4. Stream on island may still show old leader (stale).  
5. **Safety** relies on: cannot commit DCS without quorum; Postgres promote on majority side; old primary demotes on renew fail; optional disk/quorum witness.  
6. Fencing: if old primary still accepts SQL writes without demote, that is **agent bug** — design requires demote on lost renew.

### 11.3 Hub / stream down, DCS up

1. Agents still renew on Plane A.  
2. Proxies fall back to poll `GetLeader(local)` or `linearizable` rarely.  
3. No incorrect dual primary if agents obey renew.

### 11.4 CursorGone after long disconnect

1. Client reconnects with old `since_rev`.  
2. Hub returns `CursorGone`.  
3. Client requests snapshot; continues.  
4. Must not invent intermediate events.

---

## 12. API sketch (normative for implementers)

Names are conceptual; crates may use Rust types / HTTP / framed TCP.

### 12.1 Strong DCS (exists / partial)

```text
propose_dcs(DcsCommand) -> raft_index | NotLeader | CasFailed | …
dcs_get(key) -> KeyValue | None          // applied local
check_command(db, cmd) -> Ok | CasFailed // leader pre-check
```

`DcsCommand`:

- `Put { key, value, lease }`  
- `Create { key, value, lease }`  
- `Cas { key, value, expected_rev, lease }`  
- `Delete { key }`  

### 12.2 Leadership session (to build)

```text
SubscribeLeadership {
  cluster_id: bytes,
  since_revision: optional u64,
  mode: full | latest_only,
} -> stream LeadershipEvent

LeadershipEvent =
  | Snapshot { leader, rev, members, … }
  | LeaderChanged { holder, rev, … }
  | LeaderCleared { rev }
  | MemberUpsert { … }
  | MemberGone { … }
  | RoleHint { … }
  | LeaseWarning { … }
  | CursorGone { min_rev }
  | HubBye { reason }
```

### 12.3 Agent helpers

```text
TryAcquireLeader(key, holder, ttl?) -> { rev, lease? } | Lost
RenewLeader(key, holder, expected_rev) -> { rev } | LostLeadership
```

---

## 13. Mapping to current codebase (facts)

| Component | Crate / location | Status |
|-----------|------------------|--------|
| Local DCS SM | `pedradb-dcs` | Shipped |
| `DcsCommand` encode/apply | `pedradb-dcs::command` | Shipped |
| Raft + persist + TCP | `pedradb-raft`, `pedra-raft-node` | Shipped |
| `propose_dcs` / `dcs_get` | `pedradb-raft::net` | Shipped |
| Multi-process elect/put/failover tests | `pedradb-raft` tests | Shipped |
| HTTP DCS/KV | `pedradb-http` | Shipped (local DCS, not yet stream hub) |
| Leadership session hub + push | — | **Not shipped** |
| CursorGone / auto compact policy | partial doctrine | **Not fully shipped** |
| Linearizable read API | — | **Not shipped** (intentional) |
| Group fsync on Raft hot path | core has knobs; raft path sync-heavy | Improve later |
| Replicated leases with TTL | lease=0 multi-node first | Later |

---

## 14. Implementation roadmap (when building)

### P0 — Live hub MVP

1. Hook after successful `apply_dcs_command` / leader-key updates → local event log. **done** (`WatchHub` + `LeadershipHub`).  
2. In-process `subscribe_leadership` with snapshot + `LeaderChanged` (TCP SubscribeLeadership remains optional). **done**.  
3. Cursor on each event; full channel drops (non-fencing). `CursorGone` not a separate wire yet.  
4. Test: `live_hub_failover_notifies_without_polling_dcs` — subscribe; failover; event without polling DCS. **done**.

### P1 — Production-shaped

1. Member heartbeats → `MemberUpsert` / `MemberGone`.  
2. Proxy integration example (config reload on `LeaderChanged`).  
3. Agent renew + stream demote race tests.  
4. Metrics: session count, push lag, reconnects.

### P2 — Hardening

1. Cluster ID on session + RPC.  
2. Group commit / disk SLO errors.  
3. DCS history TTL compact.  
4. Optional gRPC; TLS/auth.

---

## 15. Comparison: this model vs “classic Patroni + etcd”

| Topic | Classic Patroni + etcd | This design |
|-------|------------------------|-------------|
| Source of truth | etcd key + lease | DCS on PedraDB + Raft log |
| Live view | etcd watch (often reconnect pain) | Dedicated session hub, cursor-first |
| Default read cost | Easy to overuse linearizable | Local applied + stream |
| Fencing | Lease + keys | **Revision explicit** in API |
| Multi-DCS | Patroni abstracts | Same idea; etcd not required |
| Horizontal writers | No (single raft etcd) | Future: multi-Raft KV; election remains TX/CAS |
| Footgun surface | etcd ops + poll | Doctrine + typed errors + short history |

---

## 16. DCS on Montanha-Store (target)

**Normative order:** build **Montanha-Store** (multi-Raft KV), then run DCS **as a layer** on it.

See [montanha-layering-dcs-on-store.md](montanha-layering-dcs-on-store.md).

- Leadership for a **range** = Raft leader of that range.  
- Leadership for **Postgres primary** = **transaction** on a metadata key (CAS) in the store.  
- Live hub watches applies on those keys.  
- Today’s single-domain Raft+`DcsCommand` is **bootstrap**, not the end state.

Until Store exists: **one Raft group per coordination domain** is enough for Patroni-shaped HA MVP.

---

## 17. Normative summary (checklist)

Implementations **must**:

1. Separate **Plane A (truth)** from **Plane B (live)**.  
2. Use **CAS/create + revision** for acquire/renew.  
3. Treat leadership stream as **best-effort** with **cursor reconnect**.  
4. Emit `CursorGone` instead of silent gaps.  
5. Document lag and partition staleness.  
6. Never share PedraDB directories across processes.  
7. Reject even-sized voter sets for production clusters (ops rule).

Implementations **must not**:

1. Fence primary **only** from stream events.  
2. Default all status reads to linearizable leader RPCs.  
3. Use “subscribe from now” as default reconnect.  
4. Put large application datasets in the DCS raft group.

---

## 18. Document history

| Date | Change |
|------|--------|
| 2026-08-12 | Initial detailed design: two planes, session protocol, fencing, roadmap, codebase map |

---

## 19. North-star sentence

**Patroni-shaped HA with always-on leadership sessions: strong election on PedraDB-backed DCS, live best-effort visibility of who is and is not leader, fencing always by revision — multi-node without becoming etcd.**
