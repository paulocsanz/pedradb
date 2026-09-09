#!/usr/bin/env bash
# vlog_gc_kernel.rs rustc body is the term (Aeneas VlogGc.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-core/src/vlog_gc_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_vlog_gc: verus! stand-in still in vlog_gc_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_vlog_gc: cfg(verus_keep_ghost) split still in vlog_gc_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/VlogGc.lean"
exit 0
