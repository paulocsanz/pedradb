#!/usr/bin/env bash
# Entrypoint for montanha-tcp on caixote / Linux.
#
# Modes:
#   CLUSTER=local3   — three nodes in this container (default lab smoke)
#   CLUSTER=single   — one node; set NODE_ID, BIND, PEERS
#   CLUSTER=smoke    — local3 then run smoke client against it
#
# Env:
#   DATA_DIR   (default /data)
#   NODE_ID    (single mode, default 1)
#   BIND       (single mode, default 0.0.0.0:9701)
#   PEERS      space-separated id=host:port (single mode)
#   RANGES     (default 1)

set -euo pipefail

DATA_DIR="${DATA_DIR:-/data}"
CLUSTER="${CLUSTER:-local3}"
RANGES="${RANGES:-1}"
BIN="${BIN:-montanha-tcp}"

# Hostname-based fallback: caixote sometimes starts the first boot before env is
# injected (we observed CLUSTER=netprobe in config but process still ran local3).
HN=$(hostname 2>/dev/null || true)
if [[ -z "${CLUSTER}" || "${CLUSTER}" == "local3" ]]; then
  if [[ "${HN}" == netprobe* || "${SELF_NAME:-}" == netprobe* ]]; then
    CLUSTER=netprobe
  elif [[ "${HN}" == mtcp-proxy-* || "${SELF_NAME:-}" == mtcp-proxy-* ]]; then
    CLUSTER=proxy
  elif [[ "${HN}" == mtcp-* || "${SELF_NAME:-}" == mtcp-* ]]; then
    # single-node mesh guest when name is mtcp-N-*
    if [[ "${HN}" =~ mtcp-[123]- || "${SELF_NAME:-}" =~ mtcp-[123]- ]]; then
      CLUSTER=single
    fi
  fi
fi
# Parse NODE_ID from hostname mtcp-2-n2 → 2
if [[ -z "${NODE_ID:-}" && "${HN}" =~ ^mtcp-([0-9]+)- ]]; then
  NODE_ID="${BASH_REMATCH[1]}"
fi
if [[ -z "${NODE_ID:-}" && "${SELF_NAME:-}" =~ ^mtcp-([0-9]+)- ]]; then
  NODE_ID="${BASH_REMATCH[1]}"
fi

echo "entrypoint hostname=${HN} CLUSTER=${CLUSTER} NODE_ID=${NODE_ID:-} SELF_NAME=${SELF_NAME:-}"

mkdir -p "$DATA_DIR"

start_local3() {
  local base="${PORT_BASE:-9701}"
  local p1=$base p2=$((base + 1)) p3=$((base + 2))
  local peers=(
    "--peer" "1=127.0.0.1:${p1}"
    "--peer" "2=127.0.0.1:${p2}"
    "--peer" "3=127.0.0.1:${p3}"
  )
  echo "starting local3 on :${p1}-:${p3} data=$DATA_DIR"
  "$BIN" node --id 1 --data "$DATA_DIR/n1" --bind "0.0.0.0:${p1}" --ranges "$RANGES" "${peers[@]}" &
  "$BIN" node --id 2 --data "$DATA_DIR/n2" --bind "0.0.0.0:${p2}" --ranges "$RANGES" "${peers[@]}" &
  "$BIN" node --id 3 --data "$DATA_DIR/n3" --bind "0.0.0.0:${p3}" --ranges "$RANGES" "${peers[@]}" &
  # Wait for accept
  for i in $(seq 1 60); do
    if "$BIN" status --addr "127.0.0.1:${p1}" >/dev/null 2>&1 \
      && "$BIN" status --addr "127.0.0.1:${p2}" >/dev/null 2>&1 \
      && "$BIN" status --addr "127.0.0.1:${p3}" >/dev/null 2>&1; then
      echo "local3 up"
      return 0
    fi
    sleep 0.25
  done
  echo "local3 failed to come up" >&2
  return 1
}

run_smoke() {
  local base="${PORT_BASE:-9701}"
  local p1=$base p2=$((base + 1)) p3=$((base + 2))
  "$BIN" smoke \
    --peer "1=127.0.0.1:${p1}" \
    --peer "2=127.0.0.1:${p2}" \
    --peer "3=127.0.0.1:${p3}"
}

case "$CLUSTER" in
  local3)
    start_local3
    # Keep container alive; optional periodic re-smoke via SMOKE_LOOP=1
    if [[ "${SMOKE_ON_START:-1}" == "1" ]]; then
      run_smoke || echo "initial smoke failed (cluster still up)" >&2
    fi
    if [[ "${SMOKE_LOOP:-0}" == "1" ]]; then
      while true; do
        sleep "${SMOKE_INTERVAL_SECS:-60}"
        run_smoke || true
      done
    else
      wait
    fi
    ;;
  smoke)
    start_local3
    run_smoke
    exit $?
    ;;
  single)
    NODE_ID="${NODE_ID:-1}"
    BIND="${BIND:-0.0.0.0:9701}"
    # Default health = bind port + 79 (9701 → 9780). Override with HEALTH_BIND.
    if [[ -z "${HEALTH_BIND:-}" ]]; then
      bind_port="${BIND##*:}"
      HEALTH_BIND="0.0.0.0:$((bind_port + 79))"
    fi
    # PEERS: prefer stable names (host:port / *.internal), not raw IPs when possible.
    peer_args=()
    if [[ -n "${PEERS:-}" ]]; then
      for p in $PEERS; do
        peer_args+=("--peer" "$p")
      done
    else
      peer_args+=("--peer" "${NODE_ID}=127.0.0.1:9701")
    fi
    echo "starting single id=$NODE_ID bind=$BIND health=$HEALTH_BIND peers=${PEERS:-self}"
    "$BIN" node \
      --id "$NODE_ID" \
      --data "$DATA_DIR/n${NODE_ID}" \
      --bind "$BIND" \
      --health "$HEALTH_BIND" \
      --ranges "$RANGES" \
      "${peer_args[@]}" &
    node_pid=$!
    # Guest-side mesh rewire: once L3/WG is up, push dial map to all peers so
    # host-side `mesh_wire.sh raft` is optional. Disable with AUTO_SET_PEERS=0.
    if [[ "${AUTO_SET_PEERS:-1}" == "1" && -n "${PEERS:-}" ]]; then
      (
        for i in $(seq 1 90); do
          sleep 2
          if ! kill -0 "$node_pid" 2>/dev/null; then
            exit 0
          fi
          if "$BIN" set-peers "${peer_args[@]}" 2>/dev/null; then
            echo "auto set-peers ok (try $i)"
            # Optional: wait for election noise in logs only
            "$BIN" elect-wait "${peer_args[@]}" 2>/dev/null && echo "auto elect-wait ok" || true
            exit 0
          fi
        done
        echo "auto set-peers gave up after retries (cluster may still form via CLI peers)" >&2
      ) &
    fi
    wait "$node_pid"
    ;;
  proxy)
    # RoleAware L4 TCP proxy (write→/leader, read→/follower|/ready).
    # MEMBERS: space-separated host:dataPort[@healthPort]
    # Or PEERS: id=host:port (health = dataPort+79)
    PROXY_LISTEN="${PROXY_LISTEN:-0.0.0.0:9600}"
    PROXY_MODE="${PROXY_MODE:-write}"
    proxy_args=(proxy --listen "$PROXY_LISTEN" --mode "$PROXY_MODE")
    if [[ -n "${MEMBERS:-}" ]]; then
      for m in $MEMBERS; do
        proxy_args+=(--member "$m")
      done
    elif [[ -n "${PEERS:-}" ]]; then
      for p in $PEERS; do
        # strip id= if present
        hp="${p#*=}"
        proxy_args+=(--member "$hp")
      done
    else
      echo "proxy needs MEMBERS or PEERS" >&2
      exit 1
    fi
    echo "starting proxy listen=$PROXY_LISTEN mode=$PROXY_MODE"
    exec "$BIN" "${proxy_args[@]}"
    ;;
  mesh-smoke)
    # Client-only: loop smoke against PEERS (must be real host:port after mesh_wire).
    peer_args=()
    if [[ -z "${PEERS:-}" ]]; then
      echo "mesh-smoke needs PEERS" >&2
      exit 1
    fi
    for p in $PEERS; do
      peer_args+=("--peer" "$p")
    done
    echo "mesh-smoke peers=$PEERS"
    # Wait until at least one peer answers status.
    for i in $(seq 1 120); do
      if "$BIN" elect-wait "${peer_args[@]}" 2>/dev/null; then
        break
      fi
      sleep 2
    done
    if [[ "${SMOKE_LOOP:-1}" == "1" ]]; then
      while true; do
        "$BIN" smoke "${peer_args[@]}" || echo "smoke fail (retry)" >&2
        sleep "${SMOKE_INTERVAL_SECS:-120}"
      done
    else
      exec "$BIN" smoke "${peer_args[@]}"
    fi
    ;;
  netprobe)
    # L3 canary: listen on PORT; probe PEERS and/or scan mesh /24.
    # PEERS format: "10.0.0.2:9701 10.0.0.3:9701" (space-separated, no id=).
    # Mesh IPs often change on restart — SCAN_MESH=1 (default) probes 10.0.0.100-120.
    PORT="${PORT:-9701}"
    SELF_NAME="${SELF_NAME:-netprobe}"
    SCAN_MESH="${SCAN_MESH:-1}"
    echo "netprobe start name=$SELF_NAME listen=0.0.0.0:${PORT} scan_mesh=$SCAN_MESH"
    ip -br addr 2>/dev/null || true
    ip route 2>/dev/null || true
    MY_IP=$(ip -4 -o addr show wg0 2>/dev/null | awk '{print $4}' | cut -d/ -f1 || true)
    echo "netprobe my_wg0=${MY_IP:-unknown}"
    # listener
    while true; do
      nc -l -p "$PORT" -N >/dev/null 2>&1 || nc -l -p "$PORT" >/dev/null 2>&1 || true
    done &
    LISTEN_PID=$!
    echo "netprobe listener pid=$LISTEN_PID port=$PORT"
    sleep 1

    probe_one() {
      local host="$1" port="$2"
      [[ -n "$MY_IP" && "$host" == "$MY_IP" ]] && return 0
      if nc -z -w 2 "$host" "$port" 2>/dev/null; then
        echo "netprobe OK ${SELF_NAME} -> ${host}:${port}"
        return 0
      fi
      if timeout 2 bash -c "echo >/dev/tcp/${host}/${port}" 2>/dev/null; then
        echo "netprobe OK(bash) ${SELF_NAME} -> ${host}:${port}"
        return 0
      fi
      echo "netprobe FAIL ${SELF_NAME} -> ${host}:${port}"
      return 1
    }

    while true; do
      ok_n=0
      fail_n=0
      if [[ -n "${PEERS:-}" && "${PEERS}" != "pending" ]]; then
        for target in $PEERS; do
          host="${target%:*}"
          port="${target##*:}"
          if probe_one "$host" "$port"; then ok_n=$((ok_n + 1)); else fail_n=$((fail_n + 1)); fi
        done
      fi
      if [[ "$SCAN_MESH" == "1" ]]; then
        # Scan common container mesh range without depending on stable PEERS after restart.
        for last in $(seq 100 120); do
          host="10.0.0.${last}"
          [[ -n "$MY_IP" && "$host" == "$MY_IP" ]] && continue
          if nc -z -w 1 "$host" "$PORT" 2>/dev/null; then
            echo "netprobe SCAN_OK ${SELF_NAME} -> ${host}:${PORT}"
            ok_n=$((ok_n + 1))
          fi
        done
      fi
      echo "netprobe summary ${SELF_NAME} ok=${ok_n} fail_listed=${fail_n} my=${MY_IP:-?}"
      sleep "${PROBE_INTERVAL_SECS:-5}"
    done
    ;;
  *)
    # Pass-through: montanha-tcp <args...>
    exec "$BIN" "$@"
    ;;
esac
