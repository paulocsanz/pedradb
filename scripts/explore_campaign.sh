#!/usr/bin/env bash
# RFC-0020 P1.3 — explore schedule drives in-tree campaigns.
# Rotates PEDRA_EXPLORE_OFFSET (read by silent_wrong_gate + volume_soak).
# Aggregates real silent_wrong from round reports; fails if rounds do not diverge
# or if sibling explorers exit non-zero when present.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
ROUNDS="${PEDRA_EXPLORE_ROUNDS:-4}"
LOG_DIR="${1:-$ROOT/findings/explore-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$LOG_DIR"

echo "explore_campaign rounds=$ROUNDS → $LOG_DIR"

OFFSETS=(0 17 41 99 256 777)
for ((r=0; r<ROUNDS; r++)); do
  off="${OFFSETS[$((r % ${#OFFSETS[@]}))]}"
  echo "== explore round $r offset=$off ==" | tee -a "$LOG_DIR/explore.log"
  export PEDRA_EXPLORE_OFFSET="$off"
  cargo run -q -p pedradb-dst --bin silent_wrong_gate -- \
    "$LOG_DIR/round-$r-gate.json" 2>&1 | tee -a "$LOG_DIR/round-$r-gate.log"
  PEDRA_SOAK_MODE=ops PEDRA_SOAK_TRIALS=64 cargo run -q -p pedradb-dst --bin volume_soak \
    -- "$LOG_DIR/round-$r-volume.json" 2>&1 | tee -a "$LOG_DIR/round-$r-soak.log"
done

DET="${PEDRA_DETERMINISMO_DST:-$ROOT/../determinismo/pedradb-dst}"
SIBLING_FAIL=0
if [[ -x "$DET/scripts/pedra_explore.sh" ]]; then
  echo "== sibling pedra_explore ==" | tee -a "$LOG_DIR/explore.log"
  set +e
  bash "$DET/scripts/pedra_explore.sh" 2>&1 | tee -a "$LOG_DIR/sibling-explore.log"
  EC=$?
  set -e
  if [[ "$EC" -ne 0 ]]; then
    echo "FAIL: sibling pedra_explore exited $EC" | tee -a "$LOG_DIR/explore.log" >&2
    SIBLING_FAIL=1
  fi
fi
if [[ -x "$DET/scripts/tesoura_pedra_smoke.sh" ]]; then
  echo "== sibling tesoura smoke ==" | tee -a "$LOG_DIR/explore.log"
  set +e
  bash "$DET/scripts/tesoura_pedra_smoke.sh" 2>&1 | tee -a "$LOG_DIR/sibling-tesoura.log"
  EC=$?
  set -e
  if [[ "$EC" -ne 0 ]]; then
    echo "FAIL: sibling tesoura_pedra_smoke exited $EC" | tee -a "$LOG_DIR/explore.log" >&2
    SIBLING_FAIL=1
  fi
fi

python3 - <<'PY' "$LOG_DIR" "$ROUNDS" "$SIBLING_FAIL"
import json, sys, time
from pathlib import Path

log = Path(sys.argv[1])
rounds = int(sys.argv[2])
sibling_fail = int(sys.argv[3])
sw_total = 0
seed_bases = []
offsets = []
puts = []
for r in range(rounds):
    for name in (f"round-{r}-gate.json", f"round-{r}-volume.json"):
        p = log / name
        if not p.is_file():
            print(f"FAIL: missing report {p}", file=sys.stderr)
            sys.exit(2)
        s = json.loads(p.read_text())
        sw = int(s.get("silent_wrong", 1))
        sw_total += sw
        if "volume" in name:
            seed_bases.append(int(s.get("seed_base", -1)))
            offsets.append(int(s.get("explore_offset", -1)))
            puts.append(int(s.get("puts_ok", -1)))
            print(f"round {r} volume: offset={offsets[-1]} seed_base={seed_bases[-1]} puts_ok={puts[-1]} silent_wrong={sw}")
        else:
            print(f"round {r} gate: offset={s.get('explore_offset')} seed_base={s.get('seed_base')} silent_wrong={sw}")

if sw_total != 0:
    print(f"FAIL: aggregated silent_wrong={sw_total}", file=sys.stderr)
    sys.exit(1)

# Prove explore offsets were applied and rounds diverged.
if len(set(offsets)) < min(2, rounds):
    print(f"FAIL: explore_offset did not rotate across rounds: {offsets}", file=sys.stderr)
    sys.exit(1)
if len(set(seed_bases)) < min(2, rounds):
    print(f"FAIL: seed_base identical across rounds (offset not wired): {seed_bases}", file=sys.stderr)
    sys.exit(1)
# puts_ok should typically differ with different RNG; require not all equal when rounds>1
if rounds > 1 and len(set(puts)) == 1:
    print(f"WARN: puts_ok identical {puts} — seed_base still diverged: {seed_bases}", file=sys.stderr)
    # seed_base divergence is the hard requirement; puts may collide by chance on tiny trials

if sibling_fail:
    print("FAIL: sibling explorer failed", file=sys.stderr)
    sys.exit(1)

report = {
    "gate": "P1.3-explore",
    "rounds": rounds,
    "silent_wrong": sw_total,
    "explore_offsets": offsets,
    "seed_bases": seed_bases,
    "puts_ok_by_round": puts,
    "log_dir": str(log),
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "note": "in-tree explore rotates PEDRA_EXPLORE_OFFSET into gate+soak seed_base",
}
(log / "explore_report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
print("explore_campaign OK")
PY
