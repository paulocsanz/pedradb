# The Hacker News Cynic 30-Point Audit Checklist

This checklist contains the 30 questions that elite Hacker News systems engineers, database researchers, and verification experts will ask to dismantle an ambitious technical announcement.

---

## Category A: Benchmark Rigor & Durability Physics

1. **The Peer Configuration Trap**: Did you benchmark against the peer's actual production defaults?
   - *Example:* Benchmarking against RocksDB with `sync: true` when production workloads use `sync: false` (`ROCKS_PARITY_SYNC=0`).
2. **Durability Class Alignment**: Are both engines providing identical physical durability at the moment `Ok` is returned to the client?
   - *The Trap:* Claiming your engine is faster when your engine only writes to OS page cache or user-space ring buffer, while the peer calls `fdatasync`. Pedra default is G1 (`fdatasync` before `Ok`).
3. **The Single-Client NVMe Wall**: If you claim high write throughput on single-threaded single-key puts with synchronous persistence, do the numbers violate physical laws?
   - *Reality:* Standard NVMe flash write barrier takes 80µs–500µs. A single thread with synchronous fsync cannot exceed ~2,000–12,000 ops/sec. Any claim of 500k ops/sec on a single client *must* be buffering or batching.
4. **Group Commit Under Concurrency**: Is your high throughput only achieved when many clients amortize the fsync cost via group commit? If so, is this stated explicitly?
5. **Memory Sizing vs Dataset Size**: Did the entire benchmark dataset fit in RAM?
   - *The Trap:* Running 10M keys of 100 bytes (1 GB) on a 64 GB workstation. You benchmarked `memcpy` and OS page cache read hits, not storage engine disk I/O.
6. **Key Distribution & Skew**: What key distribution was used? Sequential, Uniform Random, or Zipfian? Zipfian tests write-buffer skew and latch contention.
7. **Warm Cache vs Cold Miss Hydration**: Was the database cold-started before read benchmarks, or was the page cache completely warm?
8. **Compaction Debt & Run Duration**: Did the write benchmark run long enough to trigger full compaction cascades, write stalls, and WAL recycling? (Minimum 30–60 minutes under steady load).
9. **Hardware Environment Disclosure**: Was the test executed on bare-metal NVMe, an ephemeral VM instance, or network-attached storage (AWS EBS) with burst IOPS credits?
10. **Memory Allocation Stalls**: Were memory allocations (jemalloc locks, `realloc` during batch sorting or hydrate decompression) profiled and reported?

---

## Category B: Concurrency, Tail Latency & Synchronization Glue

11. **The Complexity Paradox & Glue Boundaries**: Are core mathematical models conflated with the synchronization glue?
    - If the LSM math is verified, what proves the correctness of `concurrent_kernel.rs` and lock-free coordination?
12. **SuperVersion Atomic Publishing**: Does compaction or flush publish the new SuperVersion atomically under the exact lock boundary where disk state transitions, or can readers witness inconsistent SST manifests?
13. **Off-Lock Compaction Races**: If compaction runs off-lock, what prevents an active reader or cleaner from observing unlinked or partially written SSTs?
14. **Group Commit Tail Latency (p99/p99.9)**: How does the engine behave under **sparse, asymmetric burst workloads**?
    - Does batch coalescing introduce long latency tails?
    - When switching between `lone_commit` (fast path for isolated writer) and group commit under sudden burst concurrency, does the ticket admission race cause latency spikes?
15. **Orphan File Invariants**: If a worker crashes or panics during an off-lock flush, does the engine detect and fail closed on uncommitted or leaked `.sst` files?
16. **Loom Scope vs Engine Scope**: Is Loom being cited as "proof that the database is race-free"?
    - *Reality:* Loom explores state spaces for isolated atomic primitives with $\le 3$ threads. It cannot verify full engine concurrency; that requires Deterministic Simulation Testing (DST) or Probabilistic Concurrency Testing (PCT).

---

## Category C: Physical I/O, VFS, & Memory Mmap Dynamics

17. **Mmap WAL & Linux VFS Interactions**: If using mmap for write logs, how does the engine behave when Linux flushes dirty pages in the background?
    - Does background writeback cause page cache write stalls or latency spikes?
18. **Fallocate Exhaustion & SIGBUS**: When disk space runs out (`ENOSPC`) on mmap growth, does the process handle `SIGBUS` gracefully or abort?
19. **Background I/O Device Queue Starvation**: Does background compaction I/O saturate NVMe controller queues and starve read latency?
20. **Crash Consistency Under Real Disk I/O**: Was crash recovery tested on real POSIX filesystems with `pwrite`/`fdatasync` and fault injection (`PEDRA_SWARM_DISK=1`), or solely on in-memory arrays?
21. **Torn Writes and Partial Blocks**: Does the write path handle power loss midway through a 4KB sector write or WAL frame?
22. **The Zero-Panic Invariant**: Can corrupt disk data, bitrot, or malicious wire inputs cause a panic (`SIGABRT`)? Are decoders using checked arithmetic (`SafeCursor`)?

---

## Category D: Drop-in RocksDB Parity & 15-Year Edge Cases

23. **Merge Operator Behavioral Parity**: Does `rocksdb-compat` handle partial merge operands with identical associativity and error semantics as RocksDB?
24. **DeleteRange Interactions**: How does `DeleteRange` interact with concurrent active iterators and compaction tombstone collapsing?
25. **Compaction Tombstone Collapsing vs Snapshot Isolation**: When compaction drops superseded keys or tombstones, is it mathematically impossible for an older active snapshot to observe data anomalies or resurrected keys?
26. **Architecture Decoupling**: Is `rocksdb-compat` isolated as a separate compatibility adapter, or has it leaked legacy C++ quirks into the pure storage kernel?

---

## Category E: Formal Verification & Proof Rigor

27. **The TCB Definition**: Is the TCB explicitly declared (Rust compiler, LLVM, Linux syscalls, hardware)?
28. **Axiom Ledger Ratchet**: How many unproven axioms (`axiom` in Lean 4 / Coq) are assumed? Are standard library methods constructive `def`s?
29. **Anti-Vacuity Testing**: Have specifications and proofs been mutation-tested with $\ge 98\%$ mutant kill score?
30. **Bit-Precision vs Infinite Abstraction**: Does the proof check machine integer limits (`u64::MAX`, `usize::MAX`) via BMC/SMT (e.g. Kani) to prevent wrapping?
