#!/usr/bin/env bash
# Extract production cqe_kernel.rs catalog entries.
# RUSTFLAGS=--cfg test so Charon sees #[cfg(test)] as_is mutants.
# Test-only AtomicU64 fetch_add in submit_complete_act is stripped back to
# the production WaitMore arm (same decision; telemetry is not the gate).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/cqe-kernel"
OUT="$ROOT/formal/aeneas/out"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

CHARON="${CHARON:-$(command -v charon || true)}"
AENEAS="${AENEAS:-$(command -v aeneas || true)}"
if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  msg="charon/aeneas not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

mkdir -p "$OUT"
SRC="$ROOT/crates/pedradb-io-uring/src/cqe_kernel.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  RUSTFLAGS="${RUSTFLAGS:-} --cfg test" "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::next_user_data' \
    --start-from 'crate::next_user_data_as_is' \
    --start-from 'crate::cqe_act' \
    --start-from 'crate::cqe_act_as_is' \
    --start-from 'crate::submit_complete_act' \
    --start-from 'crate::submit_complete_act_as_is' \
    --start-from 'crate::cqe_res_ok' \
    --start-from 'crate::cqe_res_ok_as_is' \
    --start-from 'crate::cqe_ring_model_admitted' \
    --start-from 'crate::cqe_ring_model_admitted_as_is' \
    --dest-file "$OUT/cqe_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/cqe_kernel.llbc"
python3 - "$OUT/lean/CqeKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = """def submit_complete_act
  (submit_ok : Bool) (harvested : Bool) : Result SubmitCompleteAct := do
  if harvested
  then ok SubmitCompleteAct.UseHarvested
  else
    if submit_ok
    then ok SubmitCompleteAct.WaitMore
    else
      let a ← F208_WAITMORE_AFTER_SUBMIT_ERR
      let _ ←
        core.sync.atomic.AtomicU64Align8U64.fetch_add a 1#u64
          core.sync.atomic.Ordering.Relaxed
      ok SubmitCompleteAct.WaitMore
"""
new = """def submit_complete_act
  (_submit_ok : Bool) (harvested : Bool) : Result SubmitCompleteAct := do
  if harvested
  then ok SubmitCompleteAct.UseHarvested
  else ok SubmitCompleteAct.WaitMore
"""
if old not in src:
    sys.exit("patch target not found: submit_complete_act Atomic arm")
open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
print("      patched submit_complete_act Atomic telemetry")
PYEOF
if grep -q 'sorry' "$OUT/lean/CqeKernel.lean"; then
  echo "FAIL  CqeKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-io-uring/src/cqe_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.cqe"
echo "ok    extract cqe → $OUT"
