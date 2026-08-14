#!/usr/bin/env bash
# RoleAware write-proxy smoke against a live caixote mesh (env hub netns).
#
# Usage:
#   MONTANHA_MESH_REV=m14 ./scripts/montanha_proxy_field.sh
#   HOST_SSH=paulo@192.168.68.109 MESH_NETNS=caixote-env-production-c8d3f0c9 \
#     LOCAL_BIN=/tmp/montanha-tcp-linux ./scripts/montanha_proxy_field.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST_SSH="${HOST_SSH:-paulo@192.168.68.109}"
HOST_BIN="${HOST_BIN:-/tmp/montanha-tcp}"
LOCAL_BIN="${LOCAL_BIN:-}"
TAG_SUFFIX="${MONTANHA_PROJECT_TAG_SUFFIX:-c8d3f0c9}"
MESH_NETNS="${MESH_NETNS:-caixote-env-production-${TAG_SUFFIX}}"
REV="${MONTANHA_MESH_REV:-m14}"
PROJECT="${PROJECT:-pedradb-montanha}"
PROXY_PORT="${PROXY_PORT:-9600}"
KEY="${PROXY_SMOKE_KEY:-proxy-field-$(date -u +%Y%m%dT%H%M%SZ)}"
VAL="${PROXY_SMOKE_VAL:-role-aware-ok}"

if [[ -n "$LOCAL_BIN" && -x "$LOCAL_BIN" ]]; then
  scp -q "$LOCAL_BIN" "${HOST_SSH}:${HOST_BIN}"
  ssh "$HOST_SSH" "chmod +x ${HOST_BIN}"
fi

map=$(caixote service list --project "$PROJECT" 2>/dev/null | REV="$REV" python3 -c '
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
if set(want)!={1,2,3}:
    print(f"need mtcp-{{1,2,3}}-{rev} running, have {want}", file=sys.stderr)
    sys.exit(1)
for i in (1,2,3):
    print(f"{i}|{want[i]}")
')

declare -A IP
while IFS='|' read -r i ip; do
  IP[$i]=$ip
  echo "# node $i mesh=$ip" >&2
done <<< "$map"

MEMBERS=(
  "${IP[1]}:9701"
  "${IP[2]}:9701"
  "${IP[3]}:9701"
)

remote_script=$(mktemp)
cat >"$remote_script" <<EOF
#!/bin/bash
set -euo pipefail
NS=${MESH_NETNS}
BIN=${HOST_BIN}
sudo pkill -f "montanha-tcp proxy" 2>/dev/null || true
sleep 0.3
sudo ip netns exec "\$NS" bash -c "nohup \$BIN proxy --listen 0.0.0.0:${PROXY_PORT} --mode write \\
  --member ${MEMBERS[0]} --member ${MEMBERS[1]} --member ${MEMBERS[2]} \\
  >/tmp/mtcp-proxy.log 2>&1 & echo \\\$! > /tmp/mtcp-proxy.pid"
sleep 1.2
cat /tmp/mtcp-proxy.log
sudo ip netns exec "\$NS" \$BIN put --addr 127.0.0.1:${PROXY_PORT} --key ${KEY} --value ${VAL}
for ip in ${IP[1]} ${IP[2]} ${IP[3]}; do
  echo -n "\$ip "
  sudo ip netns exec "\$NS" \$BIN get --addr \$ip:9701 --key ${KEY}
done
for ip in ${IP[1]} ${IP[2]} ${IP[3]}; do
  c=\$(sudo ip netns exec "\$NS" curl -s -o /dev/null -w '%{http_code}' --connect-timeout 1 http://\$ip:9780/leader)
  echo "\$ip /leader=\$c"
done
sudo kill "\$(cat /tmp/mtcp-proxy.pid)" 2>/dev/null || true
echo PROXY_FIELD_OK key=${KEY}
EOF

scp -q "$remote_script" "${HOST_SSH}:/tmp/mtcp-proxy-field.sh"
rm -f "$remote_script"
ssh "$HOST_SSH" "bash /tmp/mtcp-proxy-field.sh"
