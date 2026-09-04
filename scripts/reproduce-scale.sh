#!/usr/bin/env bash
# Reproduce the sorted-ingest table (Pedra always; optional Rocks / Fjall).
# One backend per process — required at 25M/100M so RSS cannot leak.
#
#   SCALE_ENTRIES=1000000 ./scripts/reproduce-scale.sh pedradb /tmp/scale-pedra
#   SCALE_ENTRIES=1000000 ./scripts/reproduce-scale.sh fjall   /tmp/scale-fjall
#   SCALE_ENTRIES=1000000 ./scripts/reproduce-scale.sh rocksdb /tmp/scale-rocks
#
# 100M: SCALE_ENTRIES=100000000 SCALE_CACHE_BYTES=268435456 TMPDIR=/data/stores \
#       ./scripts/reproduce-scale.sh pedradb /data/scale-100m-pedra
set -euo pipefail
BACKEND="${1:?backend: pedradb|fjall|rocksdb}"
OUT="${2:-findings/scale-parity/$BACKEND}"
export SCALE_BACKENDS="$BACKEND"
export SCALE_ENTRIES="${SCALE_ENTRIES:-1000000}"
export SCALE_VALUE_BYTES="${SCALE_VALUE_BYTES:-200}"
export SCALE_CACHE_BYTES="${SCALE_CACHE_BYTES:-268435456}"
FEATURES=()
case "$BACKEND" in
  pedradb) ;;
  fjall) FEATURES=(--features fjall) ;;
  rocksdb) FEATURES=(--features real) ;;
  *) echo "unknown backend $BACKEND"; exit 1 ;;
esac
exec cargo run --release -p rocksdb-parity-bench "${FEATURES[@]}" --bin scale-parity-bench -- "$OUT"
