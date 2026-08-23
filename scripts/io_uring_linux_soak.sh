#!/usr/bin/env bash
# Live-ring soak for pedradb-io-uring (audit residual: NEEDS-LINUX-ENV).
# Linux host: cargo test. Else Docker linux/arm64 or linux/amd64.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

run_tests() {
  cargo test -p pedradb-io-uring -- --nocapture
}

if [[ "$(uname -s)" == Linux ]]; then
  echo "== io_uring soak on host Linux =="
  run_tests
  echo "io_uring_linux_soak: ok (host)"
  exit 0
fi

if docker info >/dev/null 2>&1; then
  echo "== io_uring soak via Docker Linux =="
  # Docker Desktop / LinuxKit blocks io_uring_setup (EPERM) unless privileged.
  docker run --rm --privileged \
    -v "$ROOT":/src \
    -w /src \
    -e CARGO_TARGET_DIR=/tmp/pedra-iouring-target \
    rust:1-bookworm \
    cargo test -p pedradb-io-uring -- --nocapture
  echo "io_uring_linux_soak: ok (docker)"
  exit 0
fi

if [[ "${URING_SOAK_REQUIRED:-}" == 1 ]]; then
  echo "io_uring_linux_soak: Linux or Docker required (URING_SOAK_REQUIRED=1)" >&2
  exit 1
fi
echo "URING_SOAK_RESIDUAL: not Linux and no Docker"
exit 0
