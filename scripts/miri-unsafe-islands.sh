#!/usr/bin/env bash
# Miri on the unsafe islands that can actually run under Miri (audit P3).
# - pedradb-posix: real fdatasync / advise FFI (isolation off — needs the host clock + fd)
# - pedradb-io-uring cqe_kernel: unique-tag / harvest policy (pure)
#
# Not under Miri: IoUringEnv ring syscalls (need a kernel io_uring).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! cargo +nightly miri --version >/dev/null 2>&1; then
  if [[ "${MIRI_REQUIRED:-}" == 1 ]]; then
    echo "miri-unsafe-islands: nightly+miri required (MIRI_REQUIRED=1)" >&2
    exit 2
  fi
  echo "miri-unsafe-islands: nightly+miri not installed — skipping (set MIRI_REQUIRED=1 to fail)" >&2
  exit 0
fi

echo "== miri pedradb-posix (FFI, isolation off) =="
MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation}" \
  cargo +nightly miri test -p pedradb-posix -- --test-threads=1

echo "== miri pedradb-io-uring cqe_kernel =="
cargo +nightly miri test -p pedradb-io-uring cqe_kernel -- --test-threads=1

echo "miri-unsafe-islands: ok"
