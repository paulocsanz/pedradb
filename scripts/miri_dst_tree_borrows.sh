#!/usr/bin/env bash
# RFC-0052 P1.3 — same two pedradb-sim DST tests under Miri Tree Borrows.
#
# Sibling of `miri_dst_smoke.sh` (Stacked Borrows). Same allowlist: only
# `crash_after_sync_recovers_committed` and
# `failing_env_nth_put_then_reopen_recovers_prefix`. Never World/soak.
#
# RFC rule: promote this to a *required* CI job only if Tree Borrows
# ever diverges from Stacked Borrows on these tests. Until then it is
# an optional/local (or workflow_dispatch) box, not a silent no-op of
# the SB job.
#
# MIRI_REQUIRED=1: missing nightly+miri is a failure.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TB_FLAGS="-Zmiri-disable-isolation -Zmiri-tree-borrows -Zmiri-ignore-leaks"
if [[ -n "${MIRIFLAGS:-}" && "${MIRIFLAGS}" != *tree-borrows* ]]; then
  echo "miri_dst_tree_borrows: MIRIFLAGS must include -Zmiri-tree-borrows (got: ${MIRIFLAGS})" >&2
  exit 1
fi
export MIRIFLAGS="${MIRIFLAGS:-$TB_FLAGS}"

if ! cargo +nightly miri --version >/dev/null 2>&1; then
  if [[ "${MIRI_REQUIRED:-}" == 1 ]]; then
    echo "miri_dst_tree_borrows: nightly+miri required (MIRI_REQUIRED=1)" >&2
    exit 1
  fi
  echo "MIRI_RESIDUAL: install nightly+miri (or set MIRI_REQUIRED=1 in CI)"
  exit 0
fi

echo "MIRIFLAGS=${MIRIFLAGS}"
echo "== miri-tree-borrows pedradb-sim crash_after_sync_recovers_committed =="
cargo +nightly miri test -p pedradb-sim crash_after_sync_recovers_committed -- --test-threads=1

echo "== miri-tree-borrows pedradb-sim failing_env_nth_put_then_reopen_recovers_prefix =="
cargo +nightly miri test -p pedradb-sim failing_env_nth_put_then_reopen_recovers_prefix -- --test-threads=1

echo "miri_dst_tree_borrows: ok (Tree Borrows; not a Stacked Borrows rerun)"
