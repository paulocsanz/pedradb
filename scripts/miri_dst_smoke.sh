#!/usr/bin/env bash
# RFC-0057 P1 / RFC-0059 P1.2 — FailingEnv DST smoke under Miri.
#
# Two durability oracles from pedradb-sim, interpreted with Miri's
# Stacked Borrows: a commit acked after a lying sync must not reopen
# (crash-after-sync), and the nth-put EIO prefix must recover exactly
# the synced prefix on reopen. These are the engine paths the World
# harness leans on; under Miri we prove the unsafe islands they touch
# (vlog pointers, WAL frames, reopen) are UB-free in the model.
#
# MIRI_REQUIRED=1 (set in CI): a missing nightly+miri is a failure,
# not a residual note.
#
# NOTE (RFC-0052 P0.3 allowlist): this script runs *only* the two named
# pedradb-sim tests below. Do not add world_soak, silent_wrong_gate, a
# 32-seed matrix, or a full World::run — Miri is not a cluster DST box.
#
# Tree Borrows sibling: `scripts/miri_dst_tree_borrows.sh` (RFC-0052 P1.3).
# Promote that script to required CI only if it ever diverges from this one.
#
# NOTE (RFC-0052 discipline): never run Miri in the same process as
# TSan/PCT; this script is a sibling job, not a matrix combination.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! cargo +nightly miri --version >/dev/null 2>&1; then
  if [[ "${MIRI_REQUIRED:-}" == 1 ]]; then
    echo "miri_dst_smoke: nightly+miri required (MIRI_REQUIRED=1)" >&2
    exit 1
  fi
  echo "MIRI_RESIDUAL: install nightly+miri (or set MIRI_REQUIRED=1 in CI)"
  exit 0
fi

echo "== miri pedradb-sim crash_after_sync_recovers_committed =="
# -Zmiri-disable-isolation: the sim touches the host clock for unix_millis.
# -Zmiri-ignore-leaks: process-lifetime TLS (`intern_bytes`) and std
# OnceBox mutexes are not dropped at test end; this is not aliasing UB.
MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation -Zmiri-ignore-leaks}" \
  cargo +nightly miri test -p pedradb-sim crash_after_sync_recovers_committed -- --test-threads=1

echo "== miri pedradb-sim failing_env_nth_put_then_reopen_recovers_prefix =="
MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation -Zmiri-ignore-leaks}" \
  cargo +nightly miri test -p pedradb-sim failing_env_nth_put_then_reopen_recovers_prefix -- --test-threads=1

echo "miri_dst_smoke: ok"
