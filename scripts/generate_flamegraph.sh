#!/usr/bin/env bash
# ==============================================================================
# PedraDB Automated Profiler & Flamegraph Generator (macOS & Linux)
# RFC-0300 Telemetry & Diagnostics
# ==============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-"${ROOT_DIR}/findings/flamegraph-profile"}"
mkdir -p "${OUT_DIR}"

echo "================================================================================"
echo "  PedraDB Profiler & Flamegraph Engine"
echo "  Target Output: ${OUT_DIR}"
echo "================================================================================"

echo "[1/4] Compiling benchmark target with debug symbols for stack unwinding..."
CARGO_PROFILE_RELEASE_DEBUG=true cargo build --release -p rocksdb-parity-bench --bin pedra-telemetry-bench

BIN_PATH="${ROOT_DIR}/target/release/pedra-telemetry-bench"
if [[ ! -f "${BIN_PATH}" ]]; then
    echo "Error: Binary not found at ${BIN_PATH}" >&2
    exit 1
fi

echo "[2/4] Launching benchmark workload in background..."
"${BIN_PATH}" "${OUT_DIR}" > "${OUT_DIR}/bench_stdout.log" 2>&1 &
BENCH_PID=$!
echo "      Workload PID: ${BENCH_PID}"

OS="$(uname -s)"
echo "[3/4] Sampling CPU callstacks under load (OS: ${OS})..."

if [[ "${OS}" == "Darwin" ]]; then
    # macOS native sampling engine
    SAMPLE_OUT="${OUT_DIR}/sample_profile.txt"
    if kill -0 "${BENCH_PID}" 2>/dev/null; then
        echo "      Capturing 8-second thread stack sample via macOS 'sample'..."
        sample "${BENCH_PID}" 8 1 -file "${SAMPLE_OUT}" || true
    fi
else
    # Linux perf
    if command -v perf >/dev/null 2>&1; then
        echo "      Sampling via Linux 'perf'..."
        perf record -F 99 -p "${BENCH_PID}" -g -- sleep 8 || true
        perf script > "${OUT_DIR}/perf_stacks.txt" || true
    fi
fi

# Wait for benchmark to finish
wait "${BENCH_PID}" 2>/dev/null || true
echo "      Benchmark completed successfully."

echo "[4/4] Generating Flamegraph & Post-Mortem Diagnostics..."
FLAMEGRAPH_SVG="${OUT_DIR}/flamegraph.svg"

# Check if flamegraph / inferno is available
if command -v flamegraph >/dev/null 2>&1; then
    if [[ -f "${SAMPLE_OUT:-}" ]]; then
        flamegraph -o "${FLAMEGRAPH_SVG}" "${SAMPLE_OUT}" || true
    fi
fi

# Extract Top Hotspots from sample output if on macOS
if [[ -f "${SAMPLE_OUT:-}" ]]; then
    echo ""
    echo "================================================================================"
    echo "  TOP CPU HOTSPOTS DETECTED (macOS Stack Sampling)"
    echo "================================================================================"
    grep -E "([0-9]+\.[0-9]+%|[0-9]+ +Thread_)" "${SAMPLE_OUT}" | head -n 35 || true
fi

echo ""
echo "================================================================================"
echo "  PROFILING COMPLETE!"
echo "  Artifacts saved in: ${OUT_DIR}"
echo "    - Telemetry Log:     ${OUT_DIR}/telemetry.log"
echo "    - Benchmark Report:  ${OUT_DIR}/bench_report.json"
if [[ -f "${SAMPLE_OUT:-}" ]]; then
    echo "    - Callstack Sample:  ${SAMPLE_OUT}"
fi
if [[ -f "${FLAMEGRAPH_SVG}" ]]; then
    echo "    - Flamegraph SVG:    ${FLAMEGRAPH_SVG}"
fi
echo "================================================================================"
