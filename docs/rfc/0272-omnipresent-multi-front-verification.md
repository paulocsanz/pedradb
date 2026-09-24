# RFC-0272: Omnipresent Multi-Front Verification Framework (OMVF)

## 1. Executive Summary & Problem Statement

Building upon RFC-0270 (Zero-Twin Verification Policy) and RFC-0271 (Continuous Verification Chain), this RFC establishes a quadri-partite, concurrent verification framework that executes four independent, orthogonal verification pillars in parallel:

```text
┌──────────────────────────────────────────────────────────────────────────────────┐
│             RFC-0272: OMNIPRESENT MULTI-FRONT VERIFICATION (OMVF)                │
└────────────────────────────────────────┬─────────────────────────────────────────┘
                                         │
     ┌───────────────────┬───────────────┴───────────────┬───────────────────┐
     ▼                   ▼                               ▼                   ▼
┌──────────────┐  ┌──────────────┐               ┌──────────────┐  ┌──────────────┐
│   PILLAR I   │  │  PILLAR II   │               │  PILLAR III  │  │  PILLAR IV   │
│  Automated   │  │   Physical   │               │ Verus Ghost  │  │ 24/7 Soak &  │
│   Mutation   │  │   Storage    │               │    State     │  │  UCB1 Chaos  │
│   Fuzzing    │  │  Crash Seam  │               │   Weaving    │  │    Hunter    │
│ (Score ≥98%) │  │ (CrashMonkey)│               │  (Unbounded) │  │ (10^9 Ops)   │
└──────────────┘  └──────────────┘               └──────────────┘  └──────────────┘
```

By pursuing all four pillars simultaneously, PedraDB eliminates:
1. **The Human Bias in Mutation Testing:** Mutants are synthesized algorithmically across the AST, not handwritten.
2. **The 42% I/O Glue Gap:** The physical device interface (POSIX `pwrite`/`fdatasync`, `io_uring`, `mmap`) is tested under power-cut reorderings.
3. **The Bounded Unwinding Limit:** Invariants are proven for unbounded state domains ($N \in \mathbb{N}$) via Verus Ghost State.
4. **The Scale Ceiling:** Continuous 24/7 autonomous cluster soak testing driven by multi-armed bandit (UCB1) algorithms.

---

## 2. Pillar I: Automated AST Mutation Fuzzing (The Anti-Vacuity Hunter)

### 2.1 Formal Definition of Mutation Score
Given a program $P$, its specification $\mathcal{S}$, and a verification suite $\mathcal{V}$, an automated mutator $\mathcal{M}$ generates a set of mutants $\{P'_1, P'_2, \dots, P'_n\}$ where each $P'_i$ contains a single semantic fault.
The **Mutation Score** $MS$ is defined as:
$$MS = \frac{\sum_{i=1}^n \mathbb{I}[\mathcal{V}(P'_i, \mathcal{S}) = \text{FAIL}]}{n} \times 100\%$$

### 2.2 Mutation Operators
The mutator algorithmically scans all `*_kernel.rs` files and applies the following AST transformations:
1. **Relational Inversion (ROR):** `>` $\leftrightarrow$ `>=`, `<` $\leftrightarrow$ `<=`, `==` $\leftrightarrow$ `!=`.
2. **Arithmetic Off-By-One (AOR):** `x + 1` $\leftrightarrow$ `x`, `x - 1` $\leftrightarrow$ `x + 1`.
3. **Durability & Barrier Omission (DBO):** Deleting `fdatasync`, skipping CRC checksum validation, bypassing fence admission gates.
4. **Condition Inversion (COR):** Inverting `if status_is_abort` to `if !status_is_abort`.

### 2.3 Gate Policy
A hard build gate (`scripts/mutation_fuzzer.py`) automatically synthesizes mutants, executes the continuous chain, and requires:
$$\text{Mutation Score} \ge 98.0\%$$
Any surviving mutant is flagged as a specification defect.

---

## 3. Pillar II: Physical Crash Consistency & POSIX / io_uring Seam Closure

### 3.1 The Block-Level Reordering Threat
Filesystems (ext4, XFS, APFS) and NVMe write caches reorder un-flushed disk blocks out of order. If a power cut occurs before `fdatasync` issues an atomic flush barrier (`FLUSH CACHE`), the physical media may contain newer metadata pointing to older, uncommitted data blocks.

### 3.2 Seam Verification Architecture
1. **Simulated NVMe Block Device (TCG Guest & CrashMonkey):**
   Executes storage operations on a simulated block driver (`scripts/tcg_blk_eio.sh` / `scripts/rfc0229_crashmonkey_barrier_replay.sh`) that intercepts `bio` writes and tracks unflushed in-flight sectors.
2. **Power-Cut Permutation Matrix:**
   Simulates abrupt power loss at every intermediate I/O barrier, generating all physical block crash states that could be observed on physical silicon.
3. **Recovery Prefix Equivalence:**
   For every crashed block state, the recovery kernel (`pedradb_core::recover`) must restore either:
   - The exact state up to the last durable `fdatasync` barrier, OR
   - A sound historical prefix, never uncommitted future writes or corrupt records.

---

## 4. Pillar III: Verus Ghost State Weaving (Unbounded Inductive Proofs)

### 4.1 Zero-Twin Ghost State Integration
To bypass Verus's limitation with standard library types (`vstd`) without creating duplicate "twin" code:
1. **The Ghost Weaving Pattern:**
   Production code maintains standard Rust types (`std`, `parking_lot`, `bytes::Bytes`). Under `#[cfg(verus_keep_ghost)]`, ghost variables and ghost functions are woven into the production AST.
2. **Decoupled Mathematical Refinement:**
   The executable code executes at hardware speed with `#![forbid(unsafe_code)]`. The ghost code maintains mathematical sequences and sets that track the system state.
3. **Inductive Invariants:**
   Verus SMT provers prove that each executable function transition preserves the inductive invariant for all $N \ge 0$, eliminating loop unwinding limits.

---

## 5. Pillar IV: Continuous 24/7 Cluster Soak Daemon (UCB1 Chaos Hunter)

### 5.1 Architecture of the Autonomous Hunter
The binary `world_hunt_kernel` runs as a continuous daemon in the background:
1. **Multi-Armed Bandit Scheduling (UCB1):**
   The schedule space is partitioned into arms (e.g. disk corruptions, network split-brains, clock drift, extreme thread preemption). The UCB1 algorithm computes:
   $$\text{Score}(a) = \bar{X}_a + c \sqrt{\frac{\ln t}{N_a(t)}}$$
   prioritizing under-explored fault combinations and novel coverage masks.
2. **Hardware-Scale Throughput:**
   Executes at $\ge 2,500$ seeds/second across physical CPU cores, accumulating $\ge 10^8$ transactional operations per day.
3. **Automatic Crash Minimization (Shrink):**
   When an oracle detects an invariant violation, the hunter halts, invokes `world_shrink_kernel`, minimizes the event trace to the minimal reproducer, and appends the minimal seed to `findings/`.

---

## 6. Execution Plan & Enforcement

1. **Pillar I:** Automated mutation engine deployed in `scripts/mutation_fuzzer.py`.
2. **Pillar II:** CrashMonkey block replay unified into continuous verification.
3. **Pillar III:** In-tree `verus_keep_ghost` proofs verified in core kernels.
4. **Pillar IV:** Autonomous soak daemon executable via `scripts/soak_daemon.sh`.
