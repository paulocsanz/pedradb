#!/usr/bin/env bash
# Formal glue entry: caller lint, twin-token diff, clone drift, Stateright, optional Verus.
# See scripts/formal/README.md
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/formal/pedra_formal.py" "$@"
