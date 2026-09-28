#!/usr/bin/env bash
# RFC-0298: Anti-Panic & Zero-Low-Hanging-Fruit Gate Runner.
#
# Validates both static invariants and dynamic adversarial test suites:
# 1. Static Gate: Scans 220+ production source files for 0 unwrap/expect in decoders.
# 2. Unit/Fuzz Gate: Runs adversarial malformed bitstream test suites.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== [1/3] Static Anti-Panic Gate (check_anti_panic_gates.py) ==="
python3 scripts/check_anti_panic_gates.py

echo ""
echo "=== [2/3] RFC-0298 Negative Edge Fuzz Regression Gate ==="
cargo test -q -p pedradb-core --test rfc0298_safecursor_antipanic

echo ""
echo "=== [3/3] Physical Parser Greybox Fuzzer (Zero-Panic Fail-Closed) ==="
cargo test -q -p pedradb-core --test physical_parser_fuzz

echo ""
echo "=========================================================="
echo "  RFC-0298 ANTI-PANIC GATES: 100% GREEN (ALL ZERO-PANIC)  "
echo "=========================================================="
