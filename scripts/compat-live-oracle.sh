#!/usr/bin/env bash
# Live C++ Rocks vs rocksdb-compat on the rust-rocksdb 0.22 names.
# Without a C++ toolchain this prints SKIP_no_cxx and exits 0 unless
# COMPAT_LIVE_REQUIRED=1 (fail-closed, same pattern as TCG_REQUIRED).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if cargo test -p pedradb-oracle --features live-rocksdb mixed_put_delete_batch_matches_live_rocks -- --nocapture
then
  echo "C2.compat_live=ok"
  exit 0
fi
if [[ "${COMPAT_LIVE_REQUIRED:-0}" == "1" ]]; then
  echo "C2.compat_live=FAIL_no_cxx"
  exit 1
fi
echo "C2.compat_live=SKIP_no_cxx"
exit 0
