#!/usr/bin/env bash
# Lean theorems over the newly enrolled Aeneas extracts.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LEAN_DIR="$ROOT/formal/aeneas/lean"
REQUIRED=0
if [[ "${1:-}" == "--required" ]]; then
  REQUIRED=1
fi

export PATH="${HOME}/.elan/bin:${PATH}"
LAKE="${LAKE:-$(command -v lake || true)}"
if [[ -z "$LAKE" ]]; then
  msg="lake not on PATH. See formal/aeneas/PINS.md"
  if [[ "$REQUIRED" -eq 1 ]]; then
    echo "FAIL  $msg" >&2
    exit 1
  fi
  echo "skip  $msg"
  exit 0
fi

LIBS=(
  Lookup RpcMode StoreCompact Snapshot Si IndexVal Changelog
  Cursor Cl Children Pin Pack Ship Fold Manifest Compact VlogGc TxGlue
  L28 Tcg Cqe Iter Properties Scale DiskPressure Crc
  EnvCrash WalState D1Modelo WriteAck WriteAdmission GroupCommit Flush
  DcsApply StoreApply StoreCommit StoreAeAck StoreVote Key
  Lease Txn T1Modelo Membership StoreMembership C1Modelo
  CapiHandles Batch Merge FailClosed ProbeOrder Locktab Scan Cf Fields LsmR1
  Leveling Posix Form Auth Path World
)

# Cross-lib composition: import two Kernels. No generated *Kernel.lean.
COMPOSE=(
  ComposeIterMerge
  ComposeMembershipClone
  ComposeScanCrc
  ComposeC1Membership
  ComposeConcurrent
  LsmCompactCount
  ProbeLadderCount
  WorkIo
)

for lib in "${LIBS[@]}"; do
  if [[ ! -f "$LEAN_DIR/${lib}.lean" || ! -e "$LEAN_DIR/${lib}Kernel.lean" ]]; then
    echo "FAIL  formal/aeneas/lean/{${lib},${lib}Kernel}.lean missing" >&2
    exit 1
  fi
  if grep -q "sorry" "$LEAN_DIR/${lib}.lean"; then
    echo "FAIL  ${lib}.lean contains sorry" >&2
    exit 1
  fi
  if ! grep -q "^theorem " "$LEAN_DIR/${lib}.lean"; then
    echo "FAIL  ${lib}.lean has no theorem" >&2
    exit 1
  fi
done

for lib in "${COMPOSE[@]}"; do
  if [[ ! -f "$LEAN_DIR/${lib}.lean" ]]; then
    echo "FAIL  formal/aeneas/lean/${lib}.lean missing" >&2
    exit 1
  fi
  if grep -q "sorry" "$LEAN_DIR/${lib}.lean"; then
    echo "FAIL  ${lib}.lean contains sorry" >&2
    exit 1
  fi
  if ! grep -q "^theorem " "$LEAN_DIR/${lib}.lean"; then
    echo "FAIL  ${lib}.lean has no theorem" >&2
    exit 1
  fi
done

echo "      lake=$LAKE"
(cd "$LEAN_DIR" && "$LAKE" build "${LIBS[@]}" "${COMPOSE[@]}")
echo "ok    lean extracts (${#LIBS[@]} libs + ${#COMPOSE[@]} compose)"
