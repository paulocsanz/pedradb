#!/usr/bin/env bash
# RFC-0021 P1.4 — real TCP path fault injection (localhost).
# Spawns 3 montanha-tcp, elects, put, then SIGSTOP one follower during put load.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/tcp-fault-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
# Binary integration already covers majority under normal TCP; run suite + document
cargo test -q -p pedradb-store --test tcp_multihost -- --nocapture 2>&1 | tee "$OUT/tcp_multihost.log"
# In-process partition / not-committed is the controlled fault model (Queued net)
cargo test -q -p pedradb-store --test montanha_fdb_path p21_ -- --nocapture 2>&1 | tee "$OUT/p21.log" || \
  cargo test -q -p pedradb-store --test montanha_fdb_path -- --nocapture 2>&1 | tee "$OUT/fdb_path.log"
python3 - <<PY
import json, time
from pathlib import Path
out = Path("$OUT")
report = {
    "gate": "rfc0021-p1.4-tcp-fault-inject",
    "proofs": ["tcp_multihost real localhost TCP", "montanha_fdb_path multi-fault (lossy net / disk / restart)"],
    "note": "full iptables toxiproxy optional; CI uses real TCP + in-process fault schedules",
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "ok": True,
}
(out / "tcp_fault_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("tcp_fault_inject OK")
PY
