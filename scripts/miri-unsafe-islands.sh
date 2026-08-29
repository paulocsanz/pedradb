#!/usr/bin/env bash
# Miri on the unsafe islands that can actually run under Miri (audit P3).
# - pedradb-posix: real fdatasync / advise FFI (isolation off — needs the host clock + fd)
# - pedradb-io-uring cqe_kernel: unique-tag / harvest policy (pure)
# - pedradb-capi handles: slot+generation table (pure)
#
# Not under Miri: IoUringEnv ring syscalls (need a kernel io_uring),
# capi StoreCluster tests (FS + !Send cluster).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! cargo +nightly miri --version >/dev/null 2>&1; then
  if [[ "${MIRI_REQUIRED:-}" == 1 ]]; then
    echo "miri-unsafe-islands: nightly+miri required (MIRI_REQUIRED=1)" >&2
    exit 1
  fi
  echo "MIRI_RESIDUAL: install nightly+miri (or set MIRI_REQUIRED=1 in CI)"
  exit 0
fi

echo "== miri pedradb-posix fdatasync_rc_ok (RFC-0073 P1.2, safe fn) =="
cargo +nightly miri test -p pedradb-posix fdatasync_rc_ok_is_safe_predicate -- --test-threads=1

echo "== miri pedradb-posix (FFI, isolation off) =="
MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation}" \
  cargo +nightly miri test -p pedradb-posix -- --test-threads=1

echo "== miri pedradb-io-uring cqe_kernel =="
cargo +nightly miri test -p pedradb-io-uring cqe_kernel -- --test-threads=1

echo "== miri pedradb-capi handles =="
cargo +nightly miri test -p pedradb-capi handles -- --test-threads=1

echo "== miri pedradb-capi C ABI get (F210 buffer ownership) =="
# Packed handles are integer IDs (int-to-ptr). Permissive provenance is the
# model; Stacked Borrows still checks the get-buffer Box/into_raw path.
MIRIFLAGS="${MIRIFLAGS:--Zmiri-disable-isolation -Zmiri-permissive-provenance}" \
  cargo +nightly miri test -p pedradb-capi --lib tests::c_api_open_set_get_commit -- --test-threads=1

echo "== miri pedradb-capi slice caps (F215) =="
# Oversize lens never read; 4096-byte no-NUL path is an in-bounds memchr.
cargo +nightly miri test -p pedradb-capi --lib slice_cap -- --test-threads=1

echo "miri-unsafe-islands: ok"
