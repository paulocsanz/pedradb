#!/usr/bin/env bash
# Extract production key.rs (CoreError stub, no thiserror) via the shim crate.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/formal/aeneas/key-kernel"
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
SRC="$ROOT/crates/pedradb-core/src/key.rs"
echo "      charon=$CHARON"
(
  cd "$CRATE"
  "$CHARON" cargo --preset=aeneas --dest-file "$OUT/key_kernel.llbc"
)
"$AENEAS" -backend lean -dest "$OUT/lean" "$OUT/key_kernel.llbc"
{
  echo "path=crates/pedradb-core/src/key.rs"
  echo "sha256=$(shasum -a 256 "$SRC" | awk '{print $1}')"
  echo "aeneas=$("$AENEAS" -version 2>/dev/null | awk '{print $NF}')"
  echo "charon=$("$CHARON" version 2>/dev/null | head -1)"
} > "$OUT/SOURCE.key"
# Aeneas emits InternalKey Eq as a recursive impl_def; unfold like other extracts.
python3 - "$OUT/lean/KeyKernel.lean" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = (
    "@[reducible]\n"
    "impl_def key.InternalKey.Insts.CoreCmpEq : core.cmp.Eq key.InternalKey := {\n"
    "  partialEqInst := key.InternalKey.Insts.CoreCmpPartialEqInternalKey\n"
    "  assert_fields_are_eq := core.cmp.Eq.assert_fields_are_eq.default\n"
    "    key.InternalKey.Insts.CoreCmpEq\n"
    "}\n"
)
new = (
    "def key.InternalKey.Insts.CoreCmpEq.assert_fields_are_eq\n"
    "  (self : key.InternalKey) : Result Unit := do\n"
    "  ok ()\n"
    "\n"
    "@[reducible]\n"
    "def key.InternalKey.Insts.CoreCmpEq : core.cmp.Eq key.InternalKey := {\n"
    "  partialEqInst := key.InternalKey.Insts.CoreCmpPartialEqInternalKey\n"
    "  assert_fields_are_eq := key.InternalKey.Insts.CoreCmpEq.assert_fields_are_eq\n"
    "}\n"
)
if old in src:
    open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
    print("      patched InternalKey Eq impl_def")
elif new not in src:
    sys.exit("InternalKey Eq patch target not found")
PYEOF
echo "ok    extract key → $OUT"
