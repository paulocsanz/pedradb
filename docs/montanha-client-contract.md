# Montanha client contract (RFC-0017 P2.2 + RFC-0021 P0 TX + RFC-0023)

**API:** `pedradb_store::{classify, ClientClass, Transaction, PendingTx, TcpClusterClient, FdbDatabase, client_commit_tx}`

**TX parity:** [RFC-0023](rfc/0023-fdb-functional-tx-parity-and-compat-face.md) — real snapshot SI + OCC + fdb-compat.  
**Lab gates:** [RFC-0021](rfc/0021-montanha-fdb-tikv-parity-gaps.md). Not field peer.

## Error classes

| Class | When | Client |
|-------|------|--------|
| `NotLeader { leader }` | Write/strong-read hit non-leader | Retry on `leader` if `Some`, else refresh via `status` |
| `StaleLeader { live_leader }` | Strong read on deposed leader | Same |
| `NotCommitted` | Majority not reached | Backoff + retry (heal/elect) |
| `Conflict` | TX / intent conflict | App retry or abort |
| `LimitRejected` | Value > `MAX_VALUE_BYTES` (100KiB) or TX over `MAX_TX_BYTES` / `MAX_TX_KEYS` | Fix payload; do not blind-retry |
| `Unavailable` | Dial / timeout / busy | Try other peers |
| `Other` | Protocol / bug | Fail closed |

TCP `RespErr` strings use `StoreError` Display; `classify_message` parses them.

## Client TX session (RFC-0023 default = snapshot `Transaction`)

App **does not** name range leaders:

```rust
// In-process — default native API (snapshot reads + OCC)
let mut tx = cluster.begin(); // or tx_begin()
let v = tx.get(&cluster, b"a")?; // value as of begin, not concurrent commits
tx.set(b"a", b"1")?;
tx.set(b"b", b"2")?;
let tid = tx.commit(&mut cluster)?;

// Write-only weak path (no SI): cluster.pending_tx_begin()

// FDB plug/test face
let mut db = FdbDatabase::open(&mut cluster);
let mut tr = db.create_transaction();
tr.set(b"k", b"v")?;
db.commit(tr)?;

// TCP multi-host
let mut client = TcpClusterClient::new(peers);
let mut tx = client.begin_tx(); // PendingTx over CommitTx wire
tx.set(b"a", b"1")?;
let tid = client.commit_pending(tx)?; // NotLeader retry
```

**Errors:** `Conflict`, `TransactionTooOld` (watermark GC), limits.  
Limits (cluster): `MAX_VALUE_BYTES`, `MAX_TX_BYTES`, `MAX_TX_KEYS`.  
Embed Pedra-only: do **not** force FDB 5s/100KB — see RFC-0023 §Limits.

## Write retry (`TcpClusterClient::put` / `commit_tx`)

1. Dial **preferred** leader id first (cache after success).
2. On `NotLeader` with hint → set prefer → retry.
3. On miss → `client_status` all peers, parse `r1:leader=N`.
4. Cap attempts / deadline; never invent a leader without hint or status.
5. On `LimitRejected` / `Conflict` → fail closed (no silent rewrite).

## RoleAware L4 proxy (lab)

```bash
montanha-tcp proxy --listen 0.0.0.0:9600 --mode write \
  --member host1:9701 --member host2:9701 --member host3:9701
```

| `--mode` | Pick | Fail closed |
|----------|------|-------------|
| `write` | first member with `GET /leader` → 200 | yes (drop accept if none) |
| `read` | prefer `/follower` 200, else `/ready` | yes if no ready |
| `any` | any `/ready` 200 | yes if no ready |

Sideband health port defaults to **dataPort+79** (9701→9780); override with `--member host:9701@9780`.  
Clients open one TCP to the proxy listen addr; proxy splices to the chosen data backend.  
Not yet in vigia — this is the Montanha-side reference algorithm (see `proxy-raft-replicas-proposal.md`).

## Health (proxy / LB)

HTTP on `--health` (default bind+79, caixote **9780**):

| Path | 200 |
|------|-----|
| `/ready` | Worker up |
| `/leader` | This node is range leader |
| `/follower` | Up and not leader |

Use `/leader` for **write VIP**; `/ready` or `/follower` for read pools.

## Lab multi-VM

```bash
./scripts/montanha_tcp_caixote.sh wire   # set-peers + elect + smoke + /leader
```

Membership PEERS baked in image; dial map via host `set-peers` (mesh IPs).
