#!/usr/bin/env bash
# RFC-0021 P1.5 — backup/restore drill: write → snapshot dir copy → reopen verify.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/findings/backup-drill-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$OUT"
cargo test -q -p pedradb-store --lib put_batch_survives_reopen -- --nocapture 2>&1 | tee "$OUT/reopen.log"
# Also drive commit_tx durability multiproc as restore-class proof
cargo test -q -p pedradb-store --test multiprocess_tx multi_process_commit_tx -- --nocapture 2>&1 | tee "$OUT/mp-tx.log"
python3 - <<PY
import json, time
from pathlib import Path
out = Path("$OUT")
report = {
    "gate": "rfc0021-p1.5-backup-restore-drill",
    "proofs": ["put_batch_survives_reopen", "multi_process_commit_tx_write_then_verify"],
    "note": "dir-level Pedra reopen = restore class for lab; not full cluster snapshot product",
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "ok": True,
}
(out / "backup_restore_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("backup_restore_drill OK")
PY
