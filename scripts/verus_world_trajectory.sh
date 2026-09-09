#!/usr/bin/env bash
# world_kernel.rs rustc body is the term (Aeneas World.lean).
# A toy Sample/u8 view of rustc TrajectorySample is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-world/src/world_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_world_trajectory: verus! stand-in still in world_kernel.rs (not last-wins of rustc TrajectorySample)" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_world_trajectory: cfg(verus_keep_ghost) split still in world_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/World.lean"
exit 0
