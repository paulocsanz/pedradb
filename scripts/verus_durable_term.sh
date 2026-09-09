#!/usr/bin/env bash
# vote_kernel.rs rustc body is the term (Aeneas VoteKernel.lean).
# A flattened Verus stand-in of VoteInputs is not last-wins. Fail closed if it returns.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/crates/pedradb-raft/src/vote_kernel.rs"
if grep -n 'verus!' "$SRC"; then
  echo "error: verus_durable_term: verus! stand-in still in vote_kernel.rs (not last-wins of rustc VoteInputs)" >&2
  exit 1
fi
if grep -n 'verus_keep_ghost' "$SRC"; then
  echo "error: verus_durable_term: cfg(verus_keep_ghost) split still in vote_kernel.rs" >&2
  exit 1
fi
echo "ok: no Verus cartoon in $SRC; term is Aeneas formal/aeneas/lean/VoteKernel.lean"
exit 0
