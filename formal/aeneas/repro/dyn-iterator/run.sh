#!/usr/bin/env bash
# RFC-0188 P2.4 — run the minimal dyn-Trait repro against the PINNED
# Aeneas/Charon (formal/aeneas/PINS.md). Two extractions of the SAME
# crate, same pin: the control fn (concrete types) must extract; the
# repro fn (one `dyn Iterator` parameter) must be refused with
# "Dynamic trait types are not supported yet". Both verdicts are
# printed for the upstream issue; exit 0 iff both hold (refusal still
# present upstream = watch protocol re-test stays armed).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
CRATE="$ROOT/formal/aeneas/repro/dyn-iterator"
OUT="$CRATE/out"
CHARON="${CHARON:-$(command -v charon || true)}"
AENEAS="${AENEAS:-$(command -v aeneas || true)}"
if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  echo "FAIL  charon/aeneas not on PATH. See formal/aeneas/PINS.md" >&2
  exit 1
fi
rm -rf "$OUT"; mkdir -p "$OUT"

echo "      charon=$CHARON aeneas=$AENEAS"

# Pass 1 — CONTROL (concrete types only): must produce a .lean def.
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::concrete_holder_pull' \
    --dest-file "$OUT/control.llbc" ) > "$OUT/control.charon.log" 2>&1
set +e
"$AENEAS" -backend lean -dest "$OUT/control" "$OUT/control.llbc" \
  > "$OUT/control.aeneas.log" 2>&1
CONTROL_EXIT=$?
set -e
CONTROL_DEF=0
if [[ -f "$OUT/control/ReproControlKernel.lean" ]] \
   || grep -rq 'concrete_holder_pull' "$OUT/control" 2>/dev/null; then
  CONTROL_DEF=1
fi
echo "REPRO control_extracted=$CONTROL_DEF (charon+aeneas exit $CONTROL_EXIT)"

# Pass 2 — REPRO (one `dyn Iterator` parameter): Aeneas must refuse at
# the type level with the exact upstream message.
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::dyn_holder_pull' \
    --dest-file "$OUT/repro.llbc" ) > "$OUT/repro.charon.log" 2>&1
set +e
"$AENEAS" -backend lean -dest "$OUT/repro" "$OUT/repro.llbc" \
  > "$OUT/repro.aeneas.log" 2>&1
REPRO_EXIT=$?
set -e
REFUSED=0
if grep -q 'Dynamic trait types are not supported' "$OUT/repro.aeneas.log" 2>/dev/null; then
  REFUSED=1
fi
echo "REPRO dyn_trait_refused=$REFUSED (aeneas exit $REPRO_EXIT)"

if [[ "$CONTROL_DEF" == 1 && "$REFUSED" == 1 ]]; then
  echo "REPRO VERDICT: refusal reproduces on this pin (control extracts, dyn-Trait refused)"
  exit 0
fi
echo "REPRO VERDICT: MISMATCH — upstream may have opened dyn-Trait (re-run the watch protocol)" >&2
exit 2
