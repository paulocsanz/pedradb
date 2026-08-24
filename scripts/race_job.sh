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
    # RFC-0057 P1.2: two targets, two processes (never combined). The
    # second is the pct_concurrent runner WITHOUT PCT in the process
    # (RFC-0052 XOR: TSan's OS scheduler never nests with logical π).
    tsan_targets=(
      "-p pedradb-core --test concurrent_race_stress"
      "-p pedradb-world --features pct --lib pct_runner_without_pct_replays_and_covers_engine"
    )
    tsan_failed=0
    for spec in "${tsan_targets[@]}"; do
      if ! RUSTFLAGS="-Zsanitizer=thread" rustup run nightly cargo test \
        -Zbuild-std --target "$(rustc -vV | sed -n 's/^host: //p')" \
        $spec -- --nocapture; then
        if [[ "${TSAN_REQUIRED:-0}" == 1 ]]; then
          echo "tsan: $spec failed (TSAN_REQUIRED=1)" >&2
          exit 1
        fi
        tsan_failed=1
        echo "WARN: TSan $spec failed — residual documented in docs/rfc/0020 + LEDGER process"
      fi
    done
    if [[ $tsan_failed == 1 ]]; then
      echo "race_job finished with TSan WARN (see above)"
      exit 0
    fi
  else
    if [[ "${TSAN_REQUIRED:-0}" == 1 ]]; then
      echo "tsan: nightly not installed (TSAN_REQUIRED=1)" >&2
      exit 1
    fi
    echo "WARN: nightly not installed; skip TSan (stress test is authoritative in-tree job)"
  fi
else
  echo "note: set PEDRA_RUN_TSAN=1 with nightly for ThreadSanitizer path"
fi

echo "race_job OK"
