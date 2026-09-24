# RFC-0271: The Continuous Verification Chain & Systematic Anti-Vacuity Protocol

## 1. Context & Motivation

Historically, formal verification in high-assurance systems suffers from two systemic pathologies:

1. **The Semantic Silo Problem (A Fragmentação em Ilhas):**
   Tools are deployed in isolation without refinement mapping:
   - Tool A (Lean/Coq) proves an extracted pure functional fragment.
   - Tool B (Kani/CBMC) checks a bounded loop up to $k=8$.
   - Tool C (Stateright/TLA+) model-checks a simplified abstract state machine.
   - Tool D (Loom) checks a standalone concurrency mock.
   - Code $C$ (Rust) runs in production.
   There is no commutative refinement diagram connecting Lean to the binary. If any layer drifts, the formal proofs become irrelevant to the running database.

2. **The Vacuity Problem (O Problema da Vacuidade):**
   A proof or verification pass is *vacuous* if the specification passes trivially:
   - **Unsatisfiable Preconditions:** `kani::assume(P)` where $P$ is false ($P \implies Q$ is vacuously true).
   - **Tautological Assertions:** `assert!(result.is_ok() || !result.is_ok())`.
   - **Unreachable Paths:** An invariant assertion is placed in a branch that is never executed by the model checker.
   - **Lack of Mutational Sensitivity:** When a bug is deliberately injected into the code, the proof still passes.

This RFC establishes the **Continuous Verification Chain (CVC)** and the **Systematic Anti-Vacuity Protocol (SAVP)** for PedraDB.

---

## 2. The 4-Layer Continuous Verification Chain (Refinement Mapping)

Rather than five disconnected tools, verification is organized as a single, unbroken downward refinement pipeline:

```text
Layer 0: Abstract Mathematical Specification (Lean 4)
   │  Proves top-level algebraic invariants: Linearizability,
   │  Strict Serializability, Group Atomicity, Crash Monotonicity.
   │  [0 sorries, 0 axiomas].
   ▼
Layer 1: Deterministic State Kernels (*_kernel.rs)
   │  Single-artifact pure Rust functions (no_std, forbidden unsafe).
   │  - Same AST extracted by Aeneas for Lean 4.
   │  - Same AST model-checked by Stateright (global state graph).
   │  - Same AST verified by Kani (k-induction unbounded proofs).
   ▼
Layer 2: Real Concurrency AST (sync_kernel + Loom + PCT)
   │  Multi-threaded runtime orchestration wrapping Layer 1 kernels.
   │  - Concurrency primitives route through `crate::sync_kernel`.
   │  - Loom exhaustively permutes ARM64/C11 weak memory reorderings (Acquire-Release).
   │  - PCT (Probabilistic Concurrency Testing) guarantees bug detection bound 1/n^d
   │    under native OS preemption.
   ▼
Layer 3: Hardware-Scale Deterministic Simulation (DST Swarm)
   │  `world_swarm` parallel execution across all CPU cores (2,500+ seeds/sec).
   │  - Buggify fault injection (storage corruption, partition storms, power loss).
   │  - Monitored by Layer 0 mathematical oracles at runtime.
   ▼
Anti-Vacuity Gate (scripts/anti_vacuity_gate.sh)
   Verifies mutation sensitivity across all layers.
```

### Refinement Invariant
Every layer $L_{i+1}$ must be a faithful refinement of layer $L_i$:
$$\forall s \in \text{ReachableStates}(L_{i+1}), \quad \alpha(s) \in \text{ReachableStates}(L_i)$$
No mock structs or twin implementations may be introduced between layers.

---

## 3. Systematic Anti-Vacuity Protocol (SAVP)

To guarantee immunity to the vacuity problem, every verification harness must enforce three mathematical safeguards:

### 3.1 T1: Cover & Reachability Proofs
- **Kani:** Every harness containing `kani::assume(cond)` MUST include `kani::cover!(cond)`. An assumption that cannot be satisfied will cause Kani to fail the cover property, preventing vacuous truth.
- **Stateright:** Every model must specify an explicit `Property::eventually` proving that the interesting terminal states (e.g. concurrent batching, crash recovery, dirty read attempts) are actually reachable.

### 3.2 T2: Negative Mutant Sensitivity (The "Teeth" Rule)
Every verified invariant $I$ over kernel $f$ MUST possess a paired mutant $f_{\text{mutant}}$ (an "AS-IS tooth"):
$$\mathcal{V}(f, I) = \text{PASS} \quad \land \quad \mathcal{V}(f_{\text{mutant}}, I) = \text{FAIL}$$

If a mutant is verified and does NOT trigger a failure or counterexample:
$$\mathcal{V}(f_{\text{mutant}}, I) = \text{PASS} \implies \text{ABORT: SPECIFICATION VACUOUS}$$
The verification pipeline exits immediately with code 1.

### 3.3 T3: Automated Continuous Anti-Vacuity Gate
The script `scripts/anti_vacuity_gate.sh` is executed as part of CI and local verification. It runs all registered mutant teeth tests across:
1. `pedradb-store`: `txn_model`, `compact_model`, `si_model`, `snapshot_model`.
2. `pedradb-sim`: `three_teeth_plants.rs` (tombstone masking, WAL sync bypass, fsync media proof).
3. `pedradb-stream` & `pedradb-replicate`: `cursor_model`, `ship_model`.
4. `pedradb-world`: `pct_concurrent_kernel` (depth-2 vs depth-3 preemption teeth).
5. Toolchain integrity: `check_proof_check_toolchains.py` selftest.

---

## 4. Operational Invariant

No release or gate passes unless:
1. All Layer 0–3 verification stages pass on the production AST.
2. The Anti-Vacuity Gate confirms 100% of mutants produce counterexamples.
