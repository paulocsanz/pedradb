#!/usr/bin/env bash
# commit_kernel.rs rustc body is the term (Aeneas Commit.lean).
# A Verus stand-in is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/src/commit_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_commit_recover: verus! stand-in still in commit_kernel.rs" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_commit_recover: cfg(verus_keep_ghost) split still in commit_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/Commit.lean"
exit 0
