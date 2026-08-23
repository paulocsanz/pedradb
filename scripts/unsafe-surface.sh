#!/usr/bin/env bash
# Allowlist gate for Rust `unsafe` operations (audit 2026-08-22 P2).
# Allowed crates: pedradb-posix, pedradb-io-uring, pedradb-capi.
# cargo-geiger is optional (io-uring 0.7 dep TCB is expected to contain unsafe).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import os, re, sys
root = os.path.join(os.getcwd(), "crates")
allow = {"pedradb-posix", "pedradb-io-uring", "pedradb-capi"}
pat = re.compile(r"unsafe(\s+(fn|extern|impl)|\s*\{)")
hits = []
for dirpath, dirnames, files in os.walk(root):
    dirnames[:] = [d for d in dirnames if d != "target"]
    rel = os.path.relpath(dirpath, root)
    crate = rel.split(os.sep, 1)[0] if rel != "." else ""
    if crate in allow:
        continue
    for name in files:
        if not name.endswith(".rs"):
            continue
        path = os.path.join(dirpath, name)
        with open(path, encoding="utf-8", errors="replace") as f:
            for i, line in enumerate(f, 1):
                if pat.search(line):
                    hits.append(f"{path}:{i}:{line.rstrip()}")
if hits:
    print("unsafe outside allowlisted crates (posix / io-uring / capi):", file=sys.stderr)
    print("\n".join(hits), file=sys.stderr)
    sys.exit(1)
print("unsafe-surface: allowlist ok (posix, io-uring, capi)")
PY

if cargo geiger --version >/dev/null 2>&1; then
  echo "unsafe-surface: cargo geiger (report-only) on posix + io-uring + capi"
  for crate in pedradb-posix pedradb-io-uring pedradb-capi; do
    cargo geiger --manifest-path "${ROOT}/crates/${crate}/Cargo.toml" --offline || \
      echo "WARN: cargo geiger ${crate} failed (report-only)" >&2
  done
else
  echo "unsafe-surface: cargo geiger not installed (optional; allowlist is the gate)"
fi
