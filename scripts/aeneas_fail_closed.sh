#!/usr/bin/env bash
# Extract production fail_closed.rs catalog entries (parse_error_writes_status)
# plus Iterator-free F104/F105/F157/F158 gates, status/as-is, header_break
# (Windows Iterator extra `position` stripped), and Expect production walks
# (--exclude Pattern; Split clauseInst / extra all/any/position patched).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/fail-closed-kernel"
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
SRC="$ROOT/crates/pedradb-http/src/fail_closed_kernel.rs"
echo "      charon=$CHARON"
( cd "$CRATE" && "$CHARON" cargo --preset=aeneas \
    --start-from 'crate::parse_error_writes_status' \
    --start-from 'crate::parse_error_writes_status_as_is' \
    --start-from 'crate::reject_transfer_encoding' \
    --start-from 'crate::reject_transfer_encoding_as_is' \
    --start-from 'crate::present_bad_int_is_error' \
    --start-from 'crate::present_bad_int_is_error_as_is' \
    --start-from 'crate::host_values_conflict' \
    --start-from 'crate::host_values_conflict_as_is' \
    --start-from 'crate::host_value_ok' \
    --start-from 'crate::host_value_ok_as_is' \
    --start-from 'crate::parse_error_status' \
    --start-from 'crate::expectation_failed_status' \
    --start-from 'crate::expects_100_continue' \
    --start-from 'crate::expects_100_continue_as_is' \
    --start-from 'crate::expect_field_ok' \
    --start-from 'crate::expect_field_ok_as_is' \
    --start-from 'crate::http_version_requires_host' \
    --start-from 'crate::http_version_requires_host_as_is' \
    --start-from 'crate::header_break_end' \
    --start-from 'crate::header_break_end_as_is' \
    --start-from 'crate::header_break_len' \
    --exclude 'core::str::{str}::contains' \
    --exclude 'core::str::{str}::eq_ignore_ascii_case' \
    --exclude 'core::str::{str}::rsplit_once' \
    --exclude 'core::str::{str}::split_once' \
    --exclude 'core::str::{str}::find' \
    --exclude 'core::str::{str}::trim' \
    --exclude 'core::str::{str}::split' \
    --exclude 'core::str::{str}::to_ascii_uppercase' \
    --exclude 'core::str::{str}::strip_prefix' \
    --exclude 'core::str::pattern' \
    --exclude 'core::str::pattern::Pattern' \
    --dest-file "$OUT/fail_closed_kernel.llbc" )
set +e
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/fail_closed_kernel.llbc"
set -e
python3 - "$OUT/lean/FailClosedKernel.lean" <<'PYEOF'
import re
import sys

p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0

def subn(pat, new, label, count=0):
    global src, n
    src2, k = re.subn(pat, new, src, count=count)
    if count and k != count:
        sys.exit(f"patch target not found: {label} (got {k})")
    if k == 0:
        sys.exit(f"patch target not found: {label}")
    src = src2
    n += k

subn(
    r"axiom core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\.next\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  core\.str\.iter\.Split P → Result \(\(Option Str\) × \(core\.str\.iter\.Split P\)\)\n",
    "axiom core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next\n"
    "  {P : Type} :\n"
    "  core.str.iter.Split P → Result ((Option Str) × (core.str.iter.Split P))\n",
    "split next axiom",
    1,
)
subn(
    r"impl_def core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr \{P\n"
    r"  : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  core\.iter\.traits\.iterator\.Iterator \(core\.str\.iter\.Split P\) Str := \{\n"
    r"  next :=\n"
    r"    core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\.next\n"
    r"    clauseInst\n"
    r"  all := fun[\s\S]*?\n\}",
    "impl_def core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr {P\n"
    "  : Type} :\n"
    "  core.iter.traits.iterator.Iterator (core.str.iter.Split P) Str := {\n"
    "  next :=\n"
    "    core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next\n"
    "}",
    "split Iterator impl",
    1,
)
subn(
    r"axiom core\.str\.Str\.split\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(core\.str\.iter\.Split P\)\n",
    "axiom core.str.Str.split {P : Type} :\n"
    "  Str → P → Result (core.str.iter.Split P)\n",
    "split axiom",
    1,
)
subn(
    r"core\.str\.Str\.split sorry /- Could not find: trait_impl_id: \d+-/ ",
    "core.str.Str.split ",
    "split calls",
)
subn(
    r"core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\n"
    r"      sorry /- Could not find: trait_impl_id: \d+-/",
    "core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr",
    "split inst calls",
)
src2, k = re.subn(
    r"\n  (?:all|any|position|map|filter|collect|max|min) := fun[\s\S]*?(?=\n  \w+ :=|\n\})",
    "",
    src,
)
if k < 1:
    sys.exit(f"patch target not found: Iterator extra fields (got {k})")
src = src2
n += k
if "sorry" in src:
    sys.exit("FailClosedKernel.lean still contains sorry after patches")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched fail_closed ×{n}")
PYEOF
if grep -q 'sorry' "$OUT/lean/FailClosedKernel.lean"; then
  echo "FAIL  FailClosedKernel.lean still contains sorry" >&2
  exit 1
fi
{
  echo "path=crates/pedradb-http/src/fail_closed_kernel.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.fail_closed"
echo "ok    extract fail_closed → $OUT"
