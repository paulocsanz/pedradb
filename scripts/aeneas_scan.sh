#!/usr/bin/env bash
# Extract production sst/scan_kernel.rs catalog entries.
# Charon --start-from: Iterator::any closure body is Aeneas Internal error.
# Generated call_mut sorry is patched to tombstone_reaches_window (same as
# Vote Option::eq / Key Eq impl_def — generated Lean, restamped here).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/scan-kernel"
OUT="$ROOT/formal/aeneas/out"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi
CHARON="${CHARON:-$(command -v charon || true)}"
AENEAS="${AENEAS:-$(command -v aeneas || true)}"
if [[ -z "$CHARON" || -z "$AENEAS" ]]; then
  msg="charon/aeneas not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then echo "FAIL  $msg" >&2; exit 1; fi
  echo "skip  $msg"; exit 0
fi
mkdir -p "$OUT"
SRC="$ROOT/crates/pedradb-core/src/sst/scan_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::scan_kernel::scan_reads_file' \
    --start-from 'crate::scan_kernel::scan_reads_file_as_is' \
    --start-from 'crate::scan_kernel::sst_crc_fate' \
    --start-from 'crate::scan_kernel::sst_crc_fate_as_is' \
    --start-from 'crate::scan_kernel::zero_glue_admitted' \
    --start-from 'crate::scan_kernel::zero_glue_admitted_as_is' \
    --start-from 'crate::scan_kernel::sst_block_crc_ok' \
    --start-from 'crate::scan_kernel::sst_block_crc_ok_as_is' \
    --start-from 'crate::scan_kernel::key_in_window' \
    --dest-file "$OUT/scan_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/scan_kernel.llbc"
ae=$?
set -e
python3 - "$OUT/lean/ScanKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = (
    "  (tupled_args : ((Slice Std.U8) × (Slice Std.U8))) :\n"
    "  Result (Bool × scan_kernel.scan_reads_file.closure)\n"
    "  := do\n"
    "  sorry\n"
)
new = (
    "  (tupled_args : ((Slice Std.U8) × (Slice Std.U8))) :\n"
    "  Result (Bool × scan_kernel.scan_reads_file.closure)\n"
    "  := do\n"
    "  let (t_start, t_end) := tupled_args\n"
    "  let (start, end1) := c\n"
    "  let b ← scan_kernel.tombstone_reaches_window t_start t_end start end1\n"
    "  ok (b, c)\n"
)
if old in src:
    open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
    print("      patched scan_reads_file closure call_mut")
elif "let b ← scan_kernel.tombstone_reaches_window t_start t_end start end1" in src:
    print("      scan_reads_file closure already patched")
else:
    sys.exit("scan_reads_file closure patch target not found")
if "sorry" in src.replace(old, new, 1) if old in src else src:
    # re-read after write
    pass
src2 = open(p, encoding="utf-8").read()
if "\bsorry\b" in src2 or "\nsorry\n" in src2 or src2.endswith("sorry\n"):
    sys.exit("ScanKernel.lean still contains sorry")
PYEOF
if grep -q 'sorry' "$OUT/lean/ScanKernel.lean"; then
  echo "FAIL  ScanKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-core/src/sst/scan_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.scan"
echo "ok    extract scan → $OUT"
