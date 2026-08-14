#!/usr/bin/env bash
# RFC-0021 P2.1 — 24h simulation culture entrypoint (wall budget).
# Default 24h; override: PEDRA_SIM_VOLUME_WALL_SECS=86400
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PEDRA_SIM_VOLUME_WALL_SECS="${PEDRA_SIM_VOLUME_WALL_SECS:-86400}"
export PEDRA_SIM_VOLUME_ROUNDS="${PEDRA_SIM_VOLUME_ROUNDS:-1000}"
OUT="${1:-$ROOT/findings/sim-24h-$(date -u +%Y%m%dT%H%M%SZ)}"
exec bash "$ROOT/scripts/montanha_sim_volume_v0.sh" "$OUT"
