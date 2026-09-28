#!/usr/bin/env bash
# ==============================================================================
# PedraDB Benchmark, Telemetry & Post-Mortem Profiler Runner (RFC-0300)
# ==============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-"quick"}"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
OUT_DIR="${ROOT_DIR}/findings/benchmarks/${TIMESTAMP}_${MODE}"
mkdir -p "${OUT_DIR}"

echo "================================================================================"
echo "  PedraDB Universal Benchmark & Telemetry Runner"
echo "  Mode:      ${MODE} (options: quick | full | profile)"
echo "  Artifacts: ${OUT_DIR}"
echo "================================================================================"

case "${MODE}" in
    quick)
        echo "[1/2] Compiling benchmark target..."
        cargo build -p rocksdb-parity-bench --bin pedra-telemetry-bench
        echo "[2/2] Running quick benchmark suite..."
        "${ROOT_DIR}/target/debug/pedra-telemetry-bench" "${OUT_DIR}"
        ;;
    full)
        echo "[1/2] Compiling benchmark target in release mode..."
        cargo build --release -p rocksdb-parity-bench --bin pedra-telemetry-bench
        echo "[2/2] Running full release benchmark suite..."
        "${ROOT_DIR}/target/release/pedra-telemetry-bench" "${OUT_DIR}"
        ;;
    profile)
        echo "[1/2] Running benchmark under CPU stack profiler..."
        "${ROOT_DIR}/scripts/generate_flamegraph.sh" "${OUT_DIR}"
        ;;
    *)
        echo "Usage: $0 [quick | full | profile]"
        exit 1
        ;;
esac

echo ""
echo "[3/3] Generating Automated Post-Mortem Diagnostics..."
python3 "${ROOT_DIR}/scripts/analyze_telemetry.py" "${OUT_DIR}"

echo ""
echo "================================================================================"
echo "  BENCHMARK & TELEMETRY RUN COMPLETE!"
echo "  Report: ${OUT_DIR}/POSTMORTEM_PERFORMANCE_REPORT.md"
echo "================================================================================"
