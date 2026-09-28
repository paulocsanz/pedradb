# The Hacker News Cynic 30-Point Audit Checklist

This checklist contains the 30 questions that elite Hacker News systems engineers, database researchers, and verification experts will ask to dismantle an ambitious technical announcement.

---

## Category A: Benchmark Rigor & Apples-to-Oranges Traps

1. **The Peer Configuration Trap**: Did you benchmark against the peer's actual production defaults?
   - *Example:* Benchmarking against RocksDB with `sync: true` when production workloads use `sync: false` (`ROCKS_PARITY_SYNC=0`).
2. **Durability Class Alignment**: Are both engines providing identical physical durability at the moment `Ok` is returned to the client?
   - *The Trap:* Claiming your engine is faster when your engine only writes to OS page cache or user-space ring buffer, while the peer calls `fdatasync`.
3. **The Single-Client NVMe Wall**: If you claim high write throughput on single-threaded single-key puts with synchronous persistence, do the numbers violate physical laws?
   - *Reality:* Standard NVMe flash write barrier takes 80µs–500µs. A single thread with synchronous fsync cannot exceed ~2,000–12,000 ops/sec. Any claim of 500k ops/sec on a single client *must* be buffering or batching.
4. **Group Commit Under Concurrency**: Is your high throughput only achieved when many clients amortize the fsync cost via group commit? If so, is this stated explicitly?
5. **Memory Sizing vs Dataset Size**: Did the entire benchmark dataset fit in RAM?
   - *The Trap:* Running 10M keys of 100 bytes (1 GB) on a 64 GB workstation. You benchmarked `memcpy` and OS page cache read hits, not storage engine disk I/O.
6. **Key Distribution & Access Patterns**: What key distribution was used?
   - *Sequential vs Uniform Random vs Zipfian:* Sequential keys mask LSM-tree compaction cascades. Uniform random keys expose bloom filter effectiveness and cold disk read amplification. Zipfian tests write-buffer skew and latch contention.
7. **Warm Cache vs Cold Miss Hydration**: Was the database cold-started before read benchmarks, or was the page cache completely warm?
8. **Compaction Debt & Run Duration**: Did the write benchmark run long enough to trigger full compaction cascades, write stalls, and WAL recycling? (Minimum 30–60 minutes under steady load).
9. **Hardware Environment Disclosure**: Was the test executed on bare-metal NVMe, an ephemeral VM instance, or network-attached storage (AWS EBS) with burst IOPS credits?
10. **Memory Allocation Stalls**: Were memory allocations (jemalloc locks, `realloc` during batch sorting or hydrate decompression) profiled and reported?

---

## Category B: Formal Verification & Proof Denominators

11. **The TCB (Trusted Computing Base) Definition**: What is left outside the formal proof?
    - Does the proof trust the hardware, the CPU memory model, the OS kernel (POSIX syscall semantics), the Rust compiler/LLVM, the verification toolchain itself?
12. **Axiom Ledger**: How many unproven axioms (`axiom` in Lean 4 / Coq) are assumed in the formal spec?
    - Are standard library methods (e.g. `Option.is_some`, `Slice.len`, `Result.unwrap_or`) axiomatized or constructively defined?
13. **Extraction & Submodule Gaps**: Are proofs checked across all generated code, including kernel submodules?
    - *The Trap:* Checking `WriteCycle.lean` while `WriteCycleKernel.lean` hides unproven `sorry` or `admit` in error-handling paths.
14. **Anti-Vacuity Testing**: If a mutant bug is introduced into the specification or code, does the proof fail?
    - What is the mutation fuzzer score? (RFC-0273 standard: $\ge 98\%$ mutants killed).
15. **The Zero-Twin Rule**: Was the proof executed directly against production code, or against an idealized "twin/mock" model?
    - *The Trap:* Creating a simplified `MockWriteGroup` or `LoomWriteGroup` that strips away error handling, ring wrapping, and OS yields.
16. **Loom Scope vs Engine Scope**: Is Loom being cited as "proof that the database is race-free"?
    - *Reality:* Loom explores state spaces for isolated atomic primitives with $\le 3$ threads. It cannot verify full engine concurrency; that requires Deterministic Simulation Testing (DST) or Probabilistic Concurrency Testing (PCT).
17. **Model Step Budgets**: In TLA+ or Stateright models, was the transition step budget large enough to reach actual commit and liveness states?
    - *The $N+3$ Rule:* For $N$ clients in group commit, exploring $< N+3$ steps cuts off before publish/sync, generating false liveness passes.
18. **Bit-Precision vs Infinite Abstraction**: Does the proof assume unbounded mathematical integers ($\mathbb{N}, \mathbb{Z}$) that ignore machine integer overflow (`u64::MAX`, `usize::MAX`)?
    - Bit-vector bounds must be proved via SMT/BMC (e.g. Kani).

---

## Category C: Physical I/O, Parsing, & Panic Robustness

19. **The Zero-Panic Invariant**: Can corrupt disk data, bitrot, or malicious wire inputs cause a panic (`SIGABRT`)?
    - Are raw slices being parsed with `.try_into().unwrap()` instead of checked cursor decoders?
20. **Integer Overflow in Bounds Checking**: Are boundary checks calculated using addition without checking for overflow?
    - `pos + len > buf.len()` can wrap when `len = usize::MAX`, bypassing the check. Must use `pos.checked_add(len)`.
21. **Unbounded Pre-Allocation DoS**: Does the parser allocate memory based on untrusted length headers?
    - `Vec::with_capacity(n)` from a wire or disk header can trigger immediate OOM if `n` is large and remaining bytes are small.
22. **Mutex Poisoning Cascades**: Does the codebase use `.lock().unwrap()`?
    - A single thread panic can poison a central lock, causing all subsequent threads to panic in a catastrophic cascade.
23. **Crash Consistency Under Real Disk I/O**: Was crash recovery tested on real POSIX filesystems with `pwrite`/`fdatasync` and fault injection (`PEDRA_SWARM_DISK=1`), or solely on in-memory arrays?
24. **Torn Writes and Partial Blocks**: Does the write path handle power loss midway through a 4KB sector write or WAL frame?

---

## Category D: Network Protocols & Concurrency Under Adversity

25. **Slowloris & Connection Exhaustion**: Does the network server have strict read/write timeouts (`Duration::from_secs(15)`) on all accepted sockets?
26. **Frame Termination Validation (Command Smuggling)**: Does the protocol parser enforce `ensure_fully_consumed()` at the end of each frame, or are unparsed trailing bytes silently ignored?
27. **Contention & Thundering Herds**: How does the concurrency model behave under extreme contention on a single key or single range?

---

## Category E: Production Readiness & Honest Scoping

28. **Glue Code & API Trampolines**: Are contracts enforced at the glue boundaries (`db.rs`, `concurrent.rs`, POSIX wrappers), or only inside core kernels?
29. **Drop-in Compatibility Reality**: If claiming "RocksDB compatibility", does it support exact comparator semantics, column families, prefix seek, merge operators, and transaction 2PL?
30. **Failure Modes & Blast Radius**: What happens when the disk runs completely out of space (`ENOSPC`) or file descriptors are exhausted (`EMFILE`)? Does the engine fail closed cleanly without corrupting the MANIFEST?
