# Caixote multi-VM mesh: network findings

## Status (2026-08-13)

### Multi-VM Raft (**mtcp-*-m15**) — **green majority 3/3** + guest RoleAware proxy

| Service | Mesh IP | Role |
|---------|---------|------|
| mtcp-1-m15 | 10.0.0.106 | **leader** `/leader` 200 |
| mtcp-2-m15 | 10.0.0.115 | follower 503 |
| mtcp-3-m15 | 10.0.0.116 | follower 503 |
| mtcp-proxy-m15 | 10.0.0.107 | write proxy `:9600` → leader |

```bash
MONTANHA_MESH_REV=m15 LOCAL_BIN=/path/to/linux/montanha-tcp \
  ./scripts/montanha_tcp_mesh_wire.sh raft
./scripts/montanha_proxy_field.sh
# guest service path (from env hub netns):
#   put --addr 10.0.0.107:9600 → visible on all three
```

```text
smoke ok majority=3/3
GUEST_PROXY_OK  put via 10.0.0.107:9600 key guest-proxy-m15
```

Images:
- nodes: `montanha-tcp:m15-n{1,2,3}` (FROM `p02-proxy`, bake `CLUSTER=single`/`NODE_ID`)
- proxy: `montanha-tcp:m15-proxy` bake `CLUSTER=proxy` + `MEMBERS=*.internal` (env inject race)

Runtime dial map: host `set-peers` via env hub netns **or** guest `AUTO_SET_PEERS=1`.  
Deploy: `MONTANHA_MESH_PROXY=1 MONTANHA_MODE=mesh MONTANHA_REV=m15 …`

### Critical: dial from **env hub netns**, not host root

Caixote mesh for project guests is terminated on:

```text
caixote-env-production-<project8>   # e.g. caixote-env-production-c8d3f0c9
  wg-env-producti  10.0.0.1/20  listen :51850
  peer allowed-ips 10.0.0.112/32, .109, .111 …  (handshakes live)
```

**Host-root routes** to `10.0.0.x` often point at **per-service** netns
(`caixote-mtcp-*-m14-…`) whose WG peer has **no handshake** →
`nc`/`set-peers` → `No route to host` / ENETUNREACH forever.

```bash
# works:
sudo ip netns exec caixote-env-production-c8d3f0c9 \
  /tmp/montanha-tcp set-peers --peer 1=10.0.0.112:9701 …

# fails (host root → per-service veth):
/tmp/montanha-tcp set-peers --peer 1=10.0.0.112:9701 …
```

`montanha_tcp_mesh_wire.sh` defaults `MESH_NETNS=caixote-env-production-$TAG_SUFFIX`
and waits for L3 before set-peers.

Orphan netns from old revs (`m6`…`m13`, ~40) stay around with dead WG peers —
they do **not** break env-hub L3 but confuse host-root routing.

### L3 canary (netprobe **n6**) — green (before raft)

Guest↔guest TCP on mesh IPs after platform fix + recreate (not restart of dead n4 guests).

## Private DNS (do **not** hardcode `10.0.0.x` in config)

| Name form | Resolves to (live dig @10.0.0.1 in tag netns) |
|-----------|-----------------------------------------------|
| `mtcp-1-m7.internal` (no project suffix) | **NXDOMAIN / empty** |
| `mtcp-1-m7-c8d3f0c9.internal` (tag = name + project8) | **`10.0.0.1`** (gateway VIP) |
| same for mtcp-2 / mtcp-3 tags | **also `10.0.0.1`** |

Platform contract (vigia + `caixote.hosts`):

- `<tag>.internal` → **tag gateway** → vigia **LB** to replicas of that tag.
- Correct for **app → service** (e.g. `database.internal:5432`).
- **Wrong for Raft peer identity** if every peer name collapses to the same VIP / LB path: peers are not sticky pod IPs.

Lab config/scripts now default PEERS to DNS:

```text
1=mtcp-1-m7-c8d3f0c9.internal:9701
2=mtcp-2-m7-c8d3f0c9.internal:9701
3=mtcp-3-m7-c8d3f0c9.internal:9701
```

`PEERS_MODE=ip` remains a debug escape hatch only.

Successful multi-VM smoke (majority 2/3) used **mesh IPs via `set-peers`** after nodes were healthy — that was a lab proof of L3+Raft, not the product PEERS encoding.

## Ops lessons

1. **Recreate after platform networking fixes** — old netns/guests lie.
2. **First-boot env race** — containers often start as `CLUSTER=local3` / empty NODE_ID (`hostname=youki` breaks name fallback). **Restart once after env is set** before trusting mesh.
3. **Restart thrash** — can hit `Empty image reference` or IP reassignment.
4. **Never bake raw mesh IPs into IaC** — use `*.internal`; reassign-on-restart kills IP PEERS.
5. Host smoke: scp `montanha-tcp` binary to host; host does not resolve `*.internal` (guest/vigia only).

## Tag model (still true)

- Default tag = **service name** + project-id suffix.
- Multi-service L3 works **after platform fix** even across tags (n6 proved it).
- `*.internal` still gateway/LB — Raft PEERS must use **mesh IPs**.

## What works

| Path | Status |
|------|--------|
| local3 (3 proc / 1 VM) | green majority 3/3 |
| multi-service netprobe L3 | green |
| multi-VM Raft (3 singles) via **env hub** | **green majority 3/3** elect+put |
| host-root → per-service netns mesh IP | **broken** (no WG handshake) |

## Next (optional)

1. Platform: host-root routes for mesh IPs should hit env hub (or fix per-service WG).
2. GC orphan `caixote-mtcp-*` netns from old revs.
3. Vigia RoleAware pool using `:9780/leader` (lab proxy is the reference algorithm).
