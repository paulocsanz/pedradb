#!/usr/bin/env bash
# RFC-0271 — PedraDB Continuous Verification Chain (CVC) Runner.
#
# Unifies the verification layers into a single unbroken refinement pipeline:
# Stage 0: Mathematical Spec & Proof Integrity (Lean 4: 0 sorries, 0 axioms)
# Stage 1: Kernel Surface & TCB Seams (sel4_gap.py --gate)
# Stage 2: Concurrency AST under Weak Memory (Loom via crate::sync_kernel)
# Stage 3: Hardware-Scale Deterministic Simulation (DST Swarm on all CPU cores)
# Stage 4: Concurrency Race & Happen-Before Validation (scripts/race_job.sh)
# Stage 5: Systematic Anti-Vacuity Mutation Gate (scripts/anti_vacuity_gate.sh)

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "================================================================="
echo "   PedraDB Continuous Verification Chain (RFC-0271 Refinement)   "
echo "================================================================="

echo ""
echo ">>> [STAGE 0] Mathematical Proof Integrity (Lean 4)"
python3 scripts/check_lean_sorries_and_axioms.py

echo ""
echo ">>> [STAGE 1] Kernel Surface & Seams Audit (seL4 Gap Metric)"
python3 scripts/sel4_gap.py --gate

echo ""
echo ">>> [STAGE 2] Concurrency AST Verification under Weak Memory (Loom)"
# Run the fast 2-writer & reader-writer linearizability tests on real AST
cargo test -q -p pedradb-core --test loom_write_group -- loom_write_group_two_concurrent_writers
cargo test -q -p pedradb-core --test loom_write_group -- loom_write_group_reader_writer_linearizability

echo ""
echo ">>> [STAGE 3] Hardware-Scale Deterministic Simulation (DST Swarm)"
cargo run --release -q -p pedradb-world --bin world_swarm -- 2000 1 0 3 16

echo ""
echo ">>> [STAGE 4] Concurrency Race & Happens-Before Validation"
./scripts/race_job.sh

echo ""
echo ">>> [STAGE 5] Systematic Anti-Vacuity Mutation Gate"
./scripts/anti_vacuity_gate.sh

echo ""
echo ">>> [STAGE 6] Physical Disk DST Swarm on Real POSIX Media (mem=false)"
./scripts/swarm_physical_disk.sh 100 1 12

echo ""
echo ">>> [STAGE 7] Automated AST Mutation Fuzzer Gate (Score >= 98%)"
python3 scripts/mutation_fuzzer.py

echo ""
echo ">>> [STAGE 8] Physical Parser Greybox Fuzzer (Zero-Panic Fail-Closed)"
python3 scripts/physical_parser_fuzzer.py

echo ""
echo ">>> [STAGE 9] NVMe Hardware-Accurate Block-Level Crash-Consistency"
cargo test -q -p pedradb-core --test crash_consistency_nvme_replay

echo ""
echo ">>> [STAGE 10] LSM Inductive Bisimulation & FSCQ Crash-Refinement (P0/P1)"
cargo test -q -p pedradb-core --test lsm_bisimulation_invariance

echo ""
echo ">>> [STAGE 11] Lean 4 Spec Anti-Vacuity & Universalization Gate (P1)"
python3 scripts/lean_spec_vacuity_gate.py

echo ""
echo ">>> [STAGE 12] Miri Concurrency, Aliasing (Tree-Borrows) & Data-Race Gate"
./scripts/miri_concurrency_gate.sh

echo ""
echo ">>> [STAGE 13] Manifest Confluence, Liveness & SSI Serializability (RFC-0279)"
cargo test -q -p pedradb-core --test rfc0279_confluence_liveness_ssi

echo ""
echo ">>> [STAGE 14] VLog Integrity, RAM Pre-Flush Barrier, Iterator Pinning & HLC (RFC-0280)"
cargo test -q -p pedradb-core --test rfc0280_vlog_ram_iter_hlc_space

echo ""
echo ">>> [STAGE 15] Bloom Soundness, Dual-Log Recovery, Tombstone Purge & DIO (RFC-0281)"
cargo test -q -p pedradb-core --test rfc0281_bloom_dual_log_tombstone_comparator_dio

echo ""
echo ">>> [STAGE 16] Os Dez Pilares Fundamentais de Verificação Avançada (RFC-0282)"
cargo test -q -p pedradb-core --test rfc0282_dez_pilares_verificacao

echo ""
echo ">>> [STAGE 17] Os Dez Pilares da Segunda Onda de Verificação (RFC-0283)"
cargo test -q -p pedradb-core --test rfc0283_dez_pilares_segunda_onda

echo ""
echo ">>> [STAGE 18] Os Dez Pilares da Terceira Onda de Verificação (RFC-0284)"
cargo test -q -p pedradb-core --test rfc0284_dez_pilares_terceira_onda

echo ""
echo ">>> [STAGE 19] Cinco Pilares Fundamentais do Motor Puro PedraDB (RFC-0285)"
cargo test -q -p pedradb-core --test rfc0285_cinco_pilares_motor_puro

echo ""
echo ">>> [STAGE 20] Dez Pilares de Robustez Caixote / Metal / Federação (RFC-0285)"
cargo test -q -p pedradb-core --test rfc0285_dez_pilares_caixote_metal_federation

echo ""
echo "================================================================="
echo "  CONTINUOUS VERIFICATION CHAIN: 100% UNIFIED & SOUND (ALL GREEN) "
echo "================================================================="



