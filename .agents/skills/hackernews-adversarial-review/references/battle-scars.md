# Real Battle Scars: Case Studies from Systems Engineering & Verification

These case studies document real architectural vulnerabilities and verification fallacies uncovered during the development and auditing of PedraDB. Use them as concrete precedents when reviewing ambitious claims.

---

## Case Study 1: The RocksDB Parity Baseline Trap (RFC-0041)

### The Fallacy
Claiming *"Our storage engine is 3x faster than RocksDB on writes"* by configuring RocksDB with `WriteOptions.sync = true`.

### The Reality & The HN Backlash
In production, virtually nobody runs RocksDB with `sync = true` per single-write operation because synchronous `fdatasync` per write serializes on disk head/flash latency, dropping throughput to ~5,000 ops/sec. Production systems run `sync = false` (`ROCKS_PARITY_SYNC = 0`), relying on the OS page cache and background flush, or use concurrent application-level batching.

PedraDB enforces the registered product decision:
- **The Only Benchmark That Counts**: Async Pedra vs RocksDB **default** (`sync = false`).
- **Durability Asymmetry**: Pedra performs `fdatasync` before returning `Ok` (G1 default). That is the product: strictly higher durability *and* competitive speed.
- **The Single-Client Wall**: Single-client write-per-op shapes are physically below 1.0x by construction (one full hardware barrier per op vs RocksDB's zero barriers). Group commit closes and inverts the gap under concurrency (e.g. `apply_mc4` 2.788x).
- **The Rule**: Never quote single-client sync-vs-async as a win; never hide it; and never use `sync: true` peer data to claim victory. Automated gates exit with code 2 if a peer JSON has `sync: true`.

---

## Case Study 2: The Complexity Paradox & Synchronization Glue (RFC-0304)

### The Fallacy
Believing that because pure mathematical kernels (monotonic WAL sequence, key ordering, LSM levels) are formally proven in Lean 4 and Kani, the engine is free from concurrency races.

### The Reality
In real database engines, bugs almost never reside in the pure algebra of the tree. They lurk in the **synchronization glue** (`concurrent_kernel.rs`) that orchestrates off-lock I/O, SuperVersion publishing, and thread-local read caches:
1. **The Dropped Lock Gap**: A compaction job executed off-lock and took the write lock to install newly generated SSTs, but dropped the lock *before* invoking `publish_from(&g)` to update the live SuperVersion. Concurrent readers continued querying stale SST manifests or read from unlinked files.
2. **Lone Write Path Missing Publish**: Fast-path write pipelines (`submit_latched_bulk`) omitted `publish_apply`, causing concurrent reads on other threads to lag behind committed writes.
3. **The Defense**: Routing all transitions through atomic SuperVersion publishing under strict lock fences, verified via continuous differential oracles (1,000 random operations checking linearizability against reference models) and deterministic simulation testing.

---

## Case Study 3: Asymmetric Bursts & Group Commit Tail Latency Jitter

### The Fallacy
Optimizing solely for aggregate steady-state write throughput under uniform benchmark traffic.

### The Reality
Group commit pipeline shines when concurrent clients amortize `fdatasync` across dozens of operations. However, in production:
1. **The Burst Boundary**: Traffic is rarely uniform. When an isolated writer arrives, waiting for a coalescing timer window causes severe tail latency jitter (p99 / p99.9).
2. **Lone-Commit Fast-Path**: A dedicated `lone_commit` path allows isolated writers to bypass the group queue and immediately issue the barrier.
3. **Admission Races**: When sudden bursts arrive while a lone-commit is in flight, the queue must transition atomically without lock contention or thread starvation.

---

## Case Study 4: Linux VFS, Mmap WAL, and Storage Faults

### The Vulnerability Pattern
Using mmap for write logs reduces syscall overhead, but exposes the engine to Linux kernel VFS quirks:
1. **Dirty Page Writeback**: When background compaction writes dozens of megabytes, the Linux writeback flusher throttles mmap page dirtiers, generating unexpected 100ms+ latency spikes.
2. **Fallocate Exhaustion & SIGBUS**: If the filesystem runs out of space (`ENOSPC`) while touching an mmapped page, the OS sends `SIGBUS`, terminating the daemon abruptly.
3. **The Defense**: Pre-allocating log segments with explicit `fallocate`, isolating compaction I/O via rate limiters, and handling POSIX storage faults before they trigger kernel signals.

---

## Case Study 5: RocksDB-Compat & 15-Year Quirks (Tombstone Collapsing & Snapshots)

### The Fallacy
Treating compatibility as a simple wrapper around basic `get`, `put`, and `delete`.

### The Reality
Production users migrating from RocksDB rely on 15 years of implicit semantics:
1. **Merge Operators**: Associative vs partial merge operands require exact sequence ordering and accumulation semantics.
2. **DeleteRange & Iterators**: Range deletions must mask keys within their range instantaneously without invalidating active bidirectional iterators.
3. **Tombstone Collapsing vs Snapshot Isolation**: During manual or automated compaction, tombstones cannot be collapsed if any active snapshot was created prior to the tombstone's sequence number. Premature deletion resurrects superseded keys in active transactions.
4. **The Defense**: Rigorous differential test harnesses (`differential_oracle`) that fuzz randomized sequences of puts, deletes, delete_ranges, merges, and snapshots against a reference store.

---

## Case Study 6: The "100% Verificado" Fallacy & The Axiom Ratchet

### The Fallacy
Announcing *"100% formal verification with Lean 4 and zero sorries"*.

### What Was Actually Found
1. **The Bifurcated Extract Hole**: `scripts/lean_extracts.sh` checked `grep sorry $lib.lean`. However, Charon/Aeneas extracted auxiliary kernel functions into `${lib}Kernel.lean`. Three active `sorry` statements persisted in `WriteCycleKernel.lean` (error branches in WAL rotation) completely invisible to the old script.
2. **The 290 Invisible Axioms**: Charon generated Lean `axiom` declarations for standard library methods like `Option.is_some`, `Result.is_ok`, `Slice.len`, `String.is_empty`. An axiom in Lean 4 is an unverified assumption added to the TCB. A proof with 290 axioms has 290 doors open to inconsistency.

### The Remediation
- Converted standard query methods from unverified `axiom` into constructive, executable `def` functions in Lean 4.
- Pinned an explicit, ratchet-down ceiling on Lean axioms (265 max in `lean_axioms_ceiling.json`).
- Replaced superficial grep with `check_lean_sorries_and_axioms.py`, which recursively audits all 197 `.lean` files for `sorry`, `admit`, or `give_up`.

---

## Case Study 7: Stateright Step Budget Physics ($N + 3$ Rule)

### The Fallacy
Running model checking with an arbitrary step budget (e.g., 6 steps) and claiming *"Liveness and deadlock-freedom mathematically proven for concurrent clients"*.

### The Reality
When extending the concurrent write-group model from 3 to 5 clients, the model checker reported a liveness failure (`can-commit-all-clients`).

Investigation into the physical mechanics of group commit revealed:
1. $N$ steps: Each of the $N$ clients arrives and reserves a slot in the write ring.
2. $1$ step: The group leader batches the entries and allocates a WAL ticket.
3. $1$ step: The leader invokes the `fdatasync` durability barrier.
4. $1$ step: The leader publishes the sequence number and wakes the $N$ waiting clients.

$$\text{Budget}_{\min}(N) = N + 3$$

With 5 clients, an 8-step budget is the physical minimum to complete a commit cycle. A budget of 6 steps stopped before the sync/publish phase, falsely appearing as a stall or deadlock. Setting the budget to 8 steps allowed full exhaustive exploration (7,087 states, 17,584 transitions) proving strict monotonic commit without gaps.

---

## Case Study 8: RFC-0298 & The Storage Parsing Panic Epidemic

### The Vulnerability Pattern
Auditing production storage decoders (`pedradb-core`, `pedradb-store`, `pedradb-raft`) revealed direct slice indexing with `.unwrap()`. Any corrupted block on disk or adversarial payload triggered `SIGABRT` instead of an I/O error.

### The Architectural Defense (`SafeCursor`)
1. **Checked Arithmetic**: Every advance and window calculation uses `pos.checked_add(len)`.
2. **Residual Bounded Allocations**: Container capacity is strictly bounded by `cursor.remaining() / min_element_size`.
3. **Mandatory EOF Enforcement**: Network and record decoders enforce `cursor.ensure_fully_consumed()` to prevent command smuggling.
4. **Resilient Locks**: Banning `.lock().unwrap()` in shared async pools to prevent Mutex poisoning cascades.
