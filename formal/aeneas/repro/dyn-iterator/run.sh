#!/usr/bin/env bash
# Pedra Aeneas fork `pedra-dyn-struct`: type-decl of `Box<dyn Iterator>`
# extracts. Exit 0 iff the dyn holder Lean def exists (gap closed on this
# pin). Exit 2 if the old craise is back.
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

( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::concrete_holder_pull' \
    --dest-file "$OUT/control.llbc" ) > "$OUT/control.charon.log" 2>&1
set +e
"$AENEAS" -backend lean -dest "$OUT/control" "$OUT/control.llbc" \
  > "$OUT/control.aeneas.log" 2>&1
CONTROL_EXIT=$?
set -e
CONTROL_DEF=0
if grep -rq 'concrete_holder_pull' "$OUT/control" 2>/dev/null; then
  CONTROL_DEF=1
fi
echo "REPRO control_extracted=$CONTROL_DEF (aeneas exit $CONTROL_EXIT)"

( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::dyn_holder_last' \
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
DYN_DEF=0
if grep -rq 'structure DynHolder' "$OUT/repro" 2>/dev/null \
   || grep -rq 'DynHolder' "$OUT/repro" 2>/dev/null; then
  DYN_DEF=1
fi
echo "REPRO dyn_trait_refused=$REFUSED dyn_holder_extracted=$DYN_DEF (aeneas exit $REPRO_EXIT)"

if [[ "$CONTROL_DEF" == 1 && "$REFUSED" == 0 && "$DYN_DEF" == 1 ]]; then
  echo "REPRO VERDICT: type-decl gap closed on this pin (DynHolder extracts)"
  exit 0
fi
if [[ "$REFUSED" == 1 ]]; then
  echo "REPRO VERDICT: old craise is back — pin is not the pedra-dyn-struct fork" >&2
  exit 2
fi
echo "REPRO VERDICT: MISMATCH — control=$CONTROL_DEF refused=$REFUSED dyn_def=$DYN_DEF" >&2
exit 2
