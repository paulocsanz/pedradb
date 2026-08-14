#!/usr/bin/env bash
# Wire multi-container mesh PEERS using **stable private DNS** (*.internal).
#
# Default (PEERS_MODE=dns):
#   1=mtcp-1-m7-c8d3f0c9.internal:9701 …
# Platform maps <tag>.internal → tag gateway VIP (10.0.0.1 + vigia LB).
#
# Escape hatch (PEERS_MODE=ip) only for debugging L3 with raw mesh IPs from
# `caixote service list` — IPs reassign on restart; do not use as the product path.
#
# Usage:
#   MONTANHA_MESH_REV=m7 ./scripts/montanha_tcp_mesh_wire.sh raft
#   MONTANHA_MESH_REV=n6 ./scripts/montanha_tcp_mesh_wire.sh netprobe
#   PEERS_MODE=ip ./scripts/montanha_tcp_mesh_wire.sh raft   # debug only
#   ./scripts/montanha_tcp_mesh_wire.sh show
set -euo pipefail

PROJECT="${PROJECT:-pedradb-montanha}"
PORT="${PORT:-9701}"
REV="${MONTANHA_MESH_REV:-n1}"
MODE="${1:-netprobe}"
PEERS_MODE="${PEERS_MODE:-dns}"
# First 8 hex of project uuid (portaria unique_network_tags). pedradb-montanha default:
TAG_SUFFIX="${MONTANHA_PROJECT_TAG_SUFFIX:-c8d3f0c9}"

need() { command -v "$1" >/dev/null || { echo "need $1" >&2; exit 1; }; }
need caixote
need python3

json_list() {
  caixote service list --project "$PROJECT" 2>/dev/null
}

internal_host() {
  # service name → platform tag DNS: <name>-<project8>.internal
  echo "${1}-${TAG_SUFFIX}.internal"
}

# name=addr lines (addr is DNS host or IP). Prefix via env.
peer_map() {
  local prefix="$1"
  PREFIX="$prefix" PEERS_MODE="$PEERS_MODE" TAG_SUFFIX="$TAG_SUFFIX" python3 -c '
import json, os, sys
prefix = os.environ["PREFIX"]
mode = os.environ.get("PEERS_MODE", "dns")
suffix = os.environ.get("TAG_SUFFIX", "c8d3f0c9")
rows = json.load(sys.stdin)
found = []
for r in rows:
    name = r.get("service_name") or ""
    if not name.startswith(prefix):
        continue
    st = r.get("runtime_state") or r.get("status")
    if mode == "ip":
        addr = r.get("ip_v4")
        if not addr:
            print(f"missing ip for {name}", file=sys.stderr)
            sys.exit(2)
    else:
        # Prefer API private_domain when present; else name-suffix.internal
        addr = r.get("private_domain") or f"{name}-{suffix}.internal"
    found.append((name, addr, st, r.get("ip_v4")))
found.sort()
if not found:
    print(f"no services with prefix {prefix!r}", file=sys.stderr)
    sys.exit(3)
for name, addr, st, ip in found:
    print(f"# {name} -> {addr} (ip={ip} st={st})", file=sys.stderr)
    print(f"{name}={addr}")
'
}

wait_map() {
  local prefix="$1"
  local n="${2:-2}"
  echo "==> wait for >=${n} services prefix=${prefix} (mode=${PEERS_MODE})" >&2
  for i in $(seq 1 40); do
    if map=$(json_list | peer_map "$prefix" 2>/tmp/mesh-map.err); then
      cnt=$(echo "$map" | grep -c '=' || true)
      if [[ "$cnt" -ge "$n" ]]; then
        echo "$map"
        return 0
      fi
    fi
    cat /tmp/mesh-map.err >&2 2>/dev/null || true
    sleep 4
  done
  echo "timeout waiting for services" >&2
  exit 1
}

wire_netprobe() {
  local prefix="netprobe-"
  local map
  map=$(wait_map "$prefix" 2)
  echo "==> wire netprobe PEERS (mode=${PEERS_MODE})" >&2
  while IFS= read -r line; do
    [[ -z "$line" || "$line" != *"="* ]] && continue
    name="${line%%=*}"
    self="${line#*=}"
    peers=""
    while IFS= read -r line2; do
      [[ -z "$line2" || "$line2" != *"="* ]] && continue
      n2="${line2%%=*}"
      a2="${line2#*=}"
      if [[ "$n2" != "$name" && -n "$a2" ]]; then
        peers="${peers}${peers:+ }${a2}:${PORT}"
      fi
    done <<< "$map"
    echo "  $name PEERS='$peers' (self=$self)" >&2
    caixote service env set --project "$PROJECT" "$name" PEERS "$peers"
    caixote service env set --project "$PROJECT" "$name" SELF_NAME "$name"
  done <<< "$map"
  echo "==> restart netprobes (pick up env)"
  while IFS= read -r line; do
    name="${line%%=*}"
    caixote service restart --project "$PROJECT" "$name" || true
  done <<< "$map"
  echo "==> wait for netprobe OK in logs"
  sleep 15
  for i in $(seq 1 24); do
    ok=0
    while IFS= read -r line; do
      name="${line%%=*}"
      if caixote logs "$name" --limit 80 2>/dev/null | grep -qE "netprobe OK|SCAN_OK"; then
        ok=$((ok + 1))
      fi
    done <<< "$map"
    n=$(echo "$map" | wc -l | tr -d ' ')
    echo "  try $i netprobe_ok_services=$ok/$n"
    if [[ "$ok" -ge 1 ]]; then
      while IFS= read -r line; do
        name="${line%%=*}"
        echo "---- $name ----"
        caixote logs "$name" --limit 40 2>/dev/null | python3 -c '
import json,sys
try:
  d=json.load(sys.stdin)
except Exception:
  sys.exit(0)
for e in d.get("entries",[]):
  m=e.get("message","")
  if "netprobe" in m:
    print(m[:160])
' 2>/dev/null | tail -12
      done <<< "$map"
      if [[ "$ok" -eq "$n" ]]; then
        echo "L3/DNS PATH OK: all netprobes see peers"
        return 0
      fi
    fi
    sleep 5
  done
  echo "partial/fail — see logs above" >&2
  return 1
}

# Host with root routes to mesh IPs (default caixote lab host).
HOST_SSH="${HOST_SSH:-paulo@192.168.68.109}"
HOST_BIN="${HOST_BIN:-/tmp/montanha-tcp}"
LOCAL_BIN="${LOCAL_BIN:-}"
# Mesh L3 lives on the **env** hub netns (wg + allowed-ips for guest mesh IPs).
# Per-service netns (caixote-mtcp-*) often has a WG peer with no handshake; host-root
# routes to those veths return ENETUNREACH. Always dial from the env hub:
#   caixote-env-production-<project8>
MESH_NETNS="${MESH_NETNS:-caixote-env-production-${TAG_SUFFIX}}"

ensure_host_bin() {
  if [[ -n "$LOCAL_BIN" && -x "$LOCAL_BIN" ]]; then
    scp -q "$LOCAL_BIN" "${HOST_SSH}:${HOST_BIN}"
    ssh "$HOST_SSH" "chmod +x ${HOST_BIN}"
    return 0
  fi
  # Prefer release binary if present
  if [[ -x target/release/montanha-tcp ]]; then
    scp -q target/release/montanha-tcp "${HOST_SSH}:${HOST_BIN}"
    ssh "$HOST_SSH" "chmod +x ${HOST_BIN}"
    return 0
  fi
  if ssh "$HOST_SSH" "test -x ${HOST_BIN}"; then
    return 0
  fi
  echo "need montanha-tcp on host (${HOST_BIN}) or LOCAL_BIN= path" >&2
  return 1
}

# Run host binary inside MESH_NETNS (sudo ip netns exec).
host_mesh() {
  ssh "$HOST_SSH" "sudo ip netns exec ${MESH_NETNS} ${HOST_BIN} $*"
}

# Wait until mesh IPs answer TCP :PORT from the env hub (WG handshake settled).
wait_mesh_l3() {
  local ips=("$@")
  echo "==> wait L3 via netns=${MESH_NETNS} port=${PORT} ips=${ips[*]}" >&2
  for try in $(seq 1 30); do
    local ok=0
    for ip in "${ips[@]}"; do
      if ssh "$HOST_SSH" "sudo ip netns exec ${MESH_NETNS} timeout 2 bash -c 'echo >/dev/tcp/${ip}/${PORT}'" 2>/dev/null; then
        ok=$((ok + 1))
      fi
    done
    echo "  try $try l3_ok=$ok/${#ips[@]}" >&2
    if [[ "$ok" -eq "${#ips[@]}" ]]; then
      return 0
    fi
    sleep 3
  done
  echo "timeout waiting for mesh L3 via ${MESH_NETNS}" >&2
  return 1
}

# Wait until mtcp-1/2/3-$REV all running with mesh IPs.
wait_raft_ips() {
  echo "==> wait mtcp-{1,2,3}-${REV} running with mesh IPs" >&2
  for i in $(seq 1 40); do
    if map=$(json_list | REV="$REV" python3 -c '
import json,os,sys
rev=os.environ["REV"]
want={}
for r in json.load(sys.stdin):
    name=r.get("service_name") or ""
    for i in (1,2,3):
        if name==f"mtcp-{i}-{rev}":
            ip=r.get("ip_v4"); st=r.get("runtime_state") or r.get("status")
            if ip and st=="running":
                want[i]=ip
if set(want)=={1,2,3}:
    for i in (1,2,3):
        print(f"{i}|{want[i]}")
    sys.exit(0)
print(f"have {want}", file=sys.stderr)
sys.exit(1)
' 2>/tmp/raft-wait.err); then
      echo "$map"
      return 0
    fi
    cat /tmp/raft-wait.err >&2 2>/dev/null || true
    sleep 4
  done
  echo "timeout waiting for raft IPs" >&2
  exit 1
}

wire_raft() {
  # Runtime path: membership baked in image; dial map via set-peers with **mesh IPs**
  # (no restart — avoids IP thrash + empty-image bugs). DNS PEERS stay in IaC for labels.
  # Host dials from MESH_NETNS (env hub) — host-root routes to per-service netns are a trap.
  local map
  map=$(wait_raft_ips)
  declare -A IP
  while IFS='|' read -r i ip; do
    IP[$i]=$ip
    echo "# node $i mesh=${ip}" >&2
  done <<< "$map"

  ensure_host_bin || exit 1
  wait_mesh_l3 "${IP[1]}" "${IP[2]}" "${IP[3]}" || exit 1

  local peer_args=(
    --peer "1=${IP[1]}:${PORT}"
    --peer "2=${IP[2]}:${PORT}"
    --peer "3=${IP[3]}:${PORT}"
  )
  echo "==> set-peers (netns=${MESH_NETNS}) ${peer_args[*]}" >&2
  local ok=0
  for try in 1 2 3 4 5; do
    if host_mesh set-peers "${peer_args[@]}" 2>&1; then
      ok=1
      break
    fi
    echo "  set-peers try $try failed, sleep 5" >&2
    sleep 5
  done
  if [[ "$ok" -ne 1 ]]; then
    echo "set-peers failed" >&2
    exit 1
  fi

  echo "==> elect-wait" >&2
  host_mesh elect-wait "${peer_args[@]}" 2>&1

  echo "==> smoke" >&2
  host_mesh smoke "${peer_args[@]}" 2>&1

  echo "==> status + HTTP /leader" >&2
  for i in 1 2 3; do
    ip="${IP[$i]}"
    echo -n "  node$i $ip "
    host_mesh status --addr "${ip}:${PORT}" 2>&1 || true
    code=$(ssh "$HOST_SSH" "sudo ip netns exec ${MESH_NETNS} curl -s -o /tmp/mh-n${i} -w '%{http_code}' --connect-timeout 2 http://${ip}:9780/leader" 2>/dev/null || echo ERR)
    body=$(ssh "$HOST_SSH" "head -c 80 /tmp/mh-n${i} 2>/dev/null" || true)
    echo "  /leader -> $code $body"
  done
  echo "raft wire done (rev=${REV} netns=${MESH_NETNS})"
}

case "$MODE" in
  show)
    json_list | PEERS_MODE="$PEERS_MODE" TAG_SUFFIX="$TAG_SUFFIX" python3 -c '
import json,os,sys
mode=os.environ.get("PEERS_MODE","dns")
suffix=os.environ.get("TAG_SUFFIX","c8d3f0c9")
for r in json.load(sys.stdin):
  n=r.get("service_name") or ""
  ip=r.get("ip_v4")
  pd=r.get("private_domain") or (f"{n}-{suffix}.internal" if n else None)
  print(n, "dns="+str(pd), "ip="+str(ip), r.get("runtime_state"))
'
    ;;
  netprobe)
    wire_netprobe
    ;;
  raft)
    wire_raft
    ;;
  *)
    echo "usage: $0 show|netprobe|raft" >&2
    echo "  PEERS_MODE=dns|ip     (netprobe; default dns)" >&2
    echo "  MONTANHA_MESH_REV=m13 (mtcp-*-REV service suffix)" >&2
    echo "  HOST_SSH=user@host    (default paulo@192.168.68.109)" >&2
    echo "  raft: set-peers+elect+smoke+curl /leader via host binary (no restart)" >&2
    exit 2
    ;;
esac
