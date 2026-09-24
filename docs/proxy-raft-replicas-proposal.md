# Proposal: HTTP/L4 proxy for multi-service Raft (Montanha) — soviet-shaped

**Status:** lab path shipped (`montanha-tcp proxy`) · platform vigia RoleAware still design  
**Context:** 3 **distinct caixote services** (not N replicas of one service) form one Montanha Raft cluster.  
**Question:** how should private DNS + proxy route client traffic (and what *not* to do with PEERS).

### Lab: `montanha-tcp proxy` (2026-08-13)

```bash
# Write VIP: only backends with GET /leader → 200 (fail closed if none)
montanha-tcp proxy --listen 0.0.0.0:9600 --mode write \
  --member 10.0.0.112:9701 --member 10.0.0.109:9701 --member 10.0.0.111:9701

# Read: prefer /follower=200, else any /ready
montanha-tcp proxy --listen 0.0.0.0:9601 --mode read --member …

# Client
montanha-tcp put --addr 127.0.0.1:9600 --key k --value v
```

Proved: local 3-node + m14 caixote mesh (dial from `caixote-env-production-*` hub).  
Health sideband defaults to `dataPort+79` (9780).  
Field gate: `./scripts/montanha_proxy_field.sh`.  
Caixote: `MONTANHA_MESH_PROXY=1 MONTANHA_MODE=mesh` → service `mtcp-proxy-$rev` (`CLUSTER=proxy`).

---

## 1. Two different problems (do not merge them)

| Concern | Who talks | Needs | Wrong tool |
|---------|-----------|-------|------------|
| **A. Raft peer mesh** | node ↔ node | Stable **identity** (`id=host:port`), no LB | VIP `.internal` round-robin |
| **B. Client access** | app → cluster | **Role-aware** routing (writes→leader, reads→any) | Hardcoded `10.0.0.x` |

Today:

- PEERS with mesh IPs works for **A** but is ops-fragile (IP reassign).
- PEERS / clients with `*.internal` hit **gateway VIP** (`10.0.0.1`) + vigia **uniform random healthy** pool — good-ish for **B** only if *every* backend can serve the request; **broken for Raft writes** and **broken for peer identity**.

Soviet/Postgres already splits the world the same way: replication/peer traffic is **not** the same path as client SQL.

---

## 2. How soviet + platform handle Postgres replicas

### Cluster shape (RFC 0039 / soviet)

- **N containers / nodes**, not “one service scaled to 3” as the identity model.
- Soviet elects **primary**, rest **replicas**; DCS leader lock + Patroni-compatible REST.
- Portaria **does not cache primary IP** (failover makes it stale) — clients use DNS / route at connect time.

### Health API (Patroni-compatible — LB contract)

| Endpoint | 200 when | Typical LB use |
|----------|----------|----------------|
| `GET /primary` | this node is primary | **write** VIP backends |
| `GET /replica` | healthy replica (not primary) | **read** VIP backends |
| `GET /leader` | holds DCS leader lock | alternate write check |
| `GET /read-only` | not primary | read pool |
| `GET /liveness` / `/readiness` | process up | coarse filter |

HAProxy / cloud LB pattern (industry + soviet’s design intent):

```text
listen pg_write
  option httpchk GET /primary
  server n1 ... check
  server n2 ... check
  server n3 ... check
  # only current primary passes health → all write traffic goes there

listen pg_read
  option httpchk GET /replica   # or /read-only including primary
  balance roundrobin
  server n1 ... check
  server n2 ... check
  server n3 ... check
```

### What vigia does today (L4)

`BackendPool::select` = **uniform random among TCP-healthy replicas of one tag**.

- **No role** (primary/replica/leader).
- **One tag** = one service’s replicas (or one network).
- Fine for stateless HTTP workers; **not** soviet-aware; **not** Montanha-write-aware.

So: soviet already has the **algorithm surface** (`/primary` vs `/replica`); platform LB has not fully internalized role-aware pick for managed PG either — the intended product path is still dual health + dual VIP (or check-gated pool), not random.

---

## 3. Target topology for Montanha (3 services)

```text
  mtcp-1  ─┐
  mtcp-2  ─┼─ Raft PEERS (identity path, not client VIP)
  mtcp-3  ─┘

  Client ──► montanha-write.internal:port  ──► proxy ──► LEADER only
  Client ──► montanha-read.internal:port   ──► proxy ──► any healthy (or followers)
  Client ──► mtcp-N-….internal             ──► that node only (debug / ops)
```

- Keep **three services** (`mtcp-1`, `mtcp-2`, `mtcp-3`) for node identity + volumes + tags.
- Introduce a **logical cluster** object (name `montanha` / managed-cluster style) that owns:
  - member list (service ids)
  - shared Raft `scope` / cluster id
  - proxy policy (write/read)
- PEERS for Raft: stable **member DNS that is not VIP-LB**, or control-plane injected map (see §5).

---

## 4. Proxy algorithm: **RoleAware** (soviet-shaped)

### 4.1 Health / role signals (Montanha must expose)

Mirror soviet’s contract so vigia/procurador stay generic:

| Endpoint | Semantics |
|----------|-----------|
| `GET /v1/leader` or TCP admin `status` with `leader=` | **200** only if this process is range-leader (or cluster-leader for single-range lab) |
| `GET /v1/follower` | **200** if healthy follower (has leader, applying) |
| `GET /v1/ready` | process up + has peers config + not blocked |
| Optional `GET /v1/cluster` | JSON: `{ leader_id, members:[{id,host,role}] }` for control plane |

Lab already has binary status text: `local=2 members=[1,2,3] r1:leader=1` — promote to first-class health for proxy.

### 4.2 Selection policies

```text
enum RouteClass {
  Write,   // put / CAS / admin mutate
  Read,    // get / scan / status (linearizable optional)
  AnyReady // smoke / liveness only
}

fn pick(class, pool: &[Member]) -> Option<Member> {
  let ready = pool.filter(|m| m.ready && m.tcp_ok);
  match class {
    Write => {
      // 1) prefer last-known leader if still advertising /leader=200
      // 2) else scan members for /leader=200 (or status leader=self)
      // 3) else fail closed (503) — never send writes to random node
      ready.find(is_leader)
    }
    Read => {
      // default: any ready (leader + followers) — random or least-conn
      // optional strict: followers only (read-your-writes → sticky to leader)
      pick_uniform(ready)  // or least_conn
    }
    AnyReady => pick_uniform(ready),
  }
}
```

**Fail closed on Write** if no leader (election / partition). Soviet does the same for SQL on demoted primary (PG refuse writes / LB removes backend).

### 4.3 Caching & failover (latency)

| Cache | TTL | Invalidate when |
|-------|-----|-----------------|
| `leader_member_id` | 250ms–1s | `/leader` 503 on that backend, dial error, or status `leader≠self` |
| healthy set | 250ms (like current `HEALTH_CACHE_TTL`) | membership update |

On Write path miss:

1. Parallel probe `/leader` on all members (budget ≤ 2s, like soviet `is_healthiest_node` probes).
2. Pin winner until fail.
3. Optional: **on wrong-leader response** from app protocol (if Montanha returns “not leader, try N”), proxy learns and retries **once** (HAProxy `on-marked-down` style). Prefer health over app-redirect for L4.

### 4.4 HTTP vs L4

| Layer | Use |
|-------|-----|
| **L4 (vigia today)** | TCP Montanha protocol; role from **sideband health** (HTTP admin port or synthetic check). |
| **L7 (procurador HTTP)** | If/when Montanha grows HTTP API: route by method (`PUT`/`POST`→Write, `GET`→Read) or header `X-Montanha-Consistency: linearizable\|eventual`. |

Proposal: implement RoleAware first in **vigia BackendPool** (or a cluster-scoped pool), not only in HTTP edge — Montanha wire is TCP.

### 4.5 Comparison to current vigia

| | Today | Proposed RoleAware |
|--|--------|-------------------|
| Pool key | single **tag** | **cluster id** (members = services/tags) |
| Pick | random healthy | Write→leader, Read→any |
| Health | TCP connect / error budget | + **role HTTP** (`/leader`, `/follower`) |
| Fail write if no leader | may hit follower → error/timeout | **503** at proxy |

---

## 5. DNS / naming (3 services stay)

### Client (product)

| Name | Points to | Policy |
|------|-----------|--------|
| `montanha.internal` or `montanha-write.internal` | cluster VIP | **Write** (leader only) |
| `montanha-read.internal` | cluster VIP | **Read** (any / followers) |
| `mtcp-1-….internal` | **that service’s** gateway (1 replica) | direct node (debug) |

Do **not** use three different service VIPs as PEERS for Raft if each VIP is LB-to-one — PEERS need peer-to-peer mesh (§1A).

### Raft PEERS (identity)

Prefer, in order:

1. **Control-plane inject** after assign: `PEERS=1=mtcp-1:9701…` where host is **pod mesh IP or host:peer-port**, refreshed on reschedule (like not caching PG primary in portaria — inject at boot + set-peers on change).
2. **Per-member A record that is the pod IP**, not gateway VIP (platform change): `mtcp-1-….pod.internal → 10.0.0.N`.
3. Lab only: `set-peers` with mesh IPs (proven).

IaC keeps **names** (`mtcp-1`, `mtcp-2`, `mtcp-3`); runtime map is not hand-edited `10.0.0.x` in git.

---

## 6. Mapping soviet ↔ Montanha

| Soviet / PG | Montanha / Raft |
|-------------|-----------------|
| Node containers in a **cluster** | 3 services in a **cluster** |
| Primary | Range leader (lab: single range leader) |
| Replica | Follower |
| `GET /primary` | `GET /v1/leader` (or status parse) |
| `GET /replica` | `GET /v1/follower` |
| Write VIP + httpchk /primary | Write VIP + RoleAware Write |
| Read VIP + httpchk /replica | Read VIP + RoleAware Read |
| Never cache primary IP in control plane | Never bake mesh IP into IaC |
| Random L4 to any PG node | **Forbidden for writes** |

---

## 7. P0 / P1 / P2 delivery

### P0 — usable lab + honest contract

- [x] Three services (already).
- [x] Montanha: HTTP health on `--health` / `HEALTH_BIND` (default bind+79):
  - `GET /ready` `/health` → 200 if worker up
  - `GET /leader` `/primary` → 200 only if this node is range leader
  - `GET /follower` `/replica` → 200 if ready and not leader
  - `GET /status` → status_text
- [x] Docs: PEERS ≠ client VIP; client uses write/read DNS when proxy exists.
- [x] `set-peers` / control-plane PEERS refresh without git IPs (script ok).

### P1 — platform proxy

- [ ] Cluster membership list in vigia (or procurador): members from 3 services.
- [ ] `BackendPool` policy: `RoleAware { write, read }` + health URI config per port class.
- [ ] DNS: `montanha-write.internal` / `montanha-read.internal` → RoleAware VIP.
- [ ] Fail closed on write without leader; metrics: `proxy_write_no_leader`, `proxy_leader_cache_hit`.

### P2 — polish

- [ ] Least-conn / lag-aware read pick (follower lag from status).
- [ ] Linearizable read sticky to leader (`X-Consistency: linearizable`).
- [ ] Pod-IP DNS for members (`*.pod.internal`) so PEERS can be pure DNS without VIP confusion.
- [ ] Multi-range: write policy per range-leader map (Montanha multi-leader).

---

## 8. Algorithm sketch (pseudo)

```rust
// vigia or montanha-edge
struct Member {
    id: u64,
    addr: SocketAddr,
    // last probe
    ready: bool,
    is_leader: bool,
    lag: Option<u64>,
}

struct RoleAwarePool {
    members: ArcSwap<Vec<Member>>,
    leader_cache: Mutex<Option<(u64, Instant)>>,
}

impl RoleAwarePool {
    fn pick_write(&self) -> Option<SocketAddr> {
        if let Some((id, t)) = *self.leader_cache.lock() {
            if t.elapsed() < Duration::from_millis(500) {
                if let Some(m) = self.members.load().iter().find(|m| m.id == id && m.is_leader && m.ready) {
                    return Some(m.addr);
                }
            }
        }
        let leader = self.members.load().iter().find(|m| m.is_leader && m.ready)?;
        *self.leader_cache.lock() = Some((leader.id, Instant::now()));
        Some(leader.addr)
    }

    fn pick_read(&self) -> Option<SocketAddr> {
        let ready: Vec<_> = self.members.load().iter().filter(|m| m.ready).collect();
        // optional: prefer lower lag followers, fall back to leader
        pick_uniform(&ready).map(|m| m.addr)
    }
}
```

Probe loop (every 250–1000ms): for each member, TCP + `GET /leader` / `GET /ready` (or Montanha status frame).

---

## 9. What we will *not* do

- Scale **one** service to 3 replicas as the Raft topology (shared tag VIP hides identity; NODE_ID collision risk).
- Put Raft PEERS on the same VIP used for client RR.
- Send writes with today’s `pick_uniform` across all members.

---

## 10. Immediate pedradb actions (this repo)

1. Keep mesh mode as **3 services** + PEERS by **DNS name form** in IaC (stable labels).
2. Add Montanha **HTTP health** (`/leader`, `/ready`) next to TCP (so proxy can be generic like soviet).
3. Client smoke path: dial **write VIP** when RoleAware exists; until then host `set-peers` + direct mesh remains lab proof.
4. Track platform work: RoleAware pool + cluster DNS (caixote vigia/soviet patterns above).

---

## 11. One-line summary

**Soviet model:** dual health (`/primary` vs `/replica`) + dual client entrypoints; peer replication is separate.  
**Montanha model:** same split — **RoleAware proxy** (write→leader, read→any) over a **cluster of three services**; Raft PEERS stay identity-stable and never share client VIP load-balancing.
