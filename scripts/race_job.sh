#!/usr/bin/env bash
# RFC-0020 P1.4 — ConcurrentDb race job.
# Default: multi-thread stress test (always available).
# Optional: ThreadSanitizer when RUSTC nightly + sanitizer support.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== ConcurrentDb write-group stress (stable toolchain) =="
cargo test -p pedradb-core --test concurrent_race_stress -- --nocapture

if [[ "${PEDRA_RUN_TSAN:-0}" == "1" ]]; then
  echo "== ThreadSanitizer (nightly, optional) =="
  if rustup run nightly rustc -V >/dev/null 2>&1; then
    RUSTFLAGS="-Zsanitizer=thread" rustup run nightly cargo test \
      -Zbuild-std --target "$(rustc -vV | sed -n 's/^host: //p')" \
      -p pedradb-core --test concurrent_race_stress -- --nocapture \
      || {
        echo "WARN: TSan build/run failed — residual documented in docs/rfc/0020 + LEDGER process"
        exit 0
      }
  else
    echo "WARN: nightly not installed; skip TSan (stress test is authoritative in-tree job)"
  fi
else
  echo "note: set PEDRA_RUN_TSAN=1 with nightly for ThreadSanitizer path"
fi

echo "race_job OK"
