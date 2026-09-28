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
- **Durability Asymmetry**: Pedra performs `fdatasync` before returning `Ok`. That is the product: strictly higher durability *and* competitive speed.
- **The Single-Client Wall**: Single-client write-per-op shapes are physically below 1.0x by construction (one full hardware barrier per op vs RocksDB's zero barriers). Group commit only closes the gap under concurrency (e.g. `apply_mc4` 2.788x).
- **The Rule**: Never quote single-client sync-vs-async as a win; never hide it; and never use `sync: true` peer data to claim victory. Automated gates exit with code 2 if a peer JSON has `sync: true`.

---

## Case Study 2: The "100% Verificado" Fallacy & The Axiom Ratchet

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

## Case Study 3: Stateright Step Budget Physics ($N + 3$ Rule)

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

## Case Study 4: RFC-0298 & The Storage Parsing Panic Epidemic

### The Vulnerability Pattern
Auditing production storage decoders (`pedradb-core`, `pedradb-store`, `pedradb-raft`) revealed a widespread vulnerability:
```rust
// VULNERABLE: Direct slice indexing + unwrap()
let len = u32::from_le_bytes(buf[pos..pos+4].try_into().unwrap()) as usize;
let data = &buf[pos+4..pos+4+len];
```
Any corrupted block on disk, partial network packet, or adversarial payload triggered `SIGABRT` (immediate daemon crash) instead of an I/O error.

### Integer Overflow Wrap in Slice Arithmetic
```rust
// VULNERABLE: Integer wrap
if pos + len > buf.len() { return Err(...); }
// If len == usize::MAX, pos + len wraps around to 0, passing the check!
```

### Unbounded Pre-Allocation Memory Exhaustion
```rust
// VULNERABLE: OOM DoS
let count = cursor.read_u32()? as usize;
let mut items = Vec::with_capacity(count); // Allocates gigabytes if count = 0xFFFFFFFF
```

### The Architectural Defense (`SafeCursor`)
1. **Checked Arithmetic**: Every advance and window calculation uses `pos.checked_add(len)`.
2. **Residual Bounded Allocations**: Container capacity is strictly bounded by `cursor.remaining() / min_element_size`.
3. **Mandatory EOF Enforcement**: Network and record decoders enforce `cursor.ensure_fully_consumed()` to prevent command smuggling and unparsed trailing garbage.
4. **Resilient Locks**: Banning `.lock().unwrap()` in shared async pools to prevent Mutex poisoning cascades.

---

## Case Study 5: The Zero-Twin Policy & Anti-Vacuity (RFC-0270 & RFC-0273)

### The Fallacy
Writing a simplified "model twin" (e.g. `LoomWriteGroup` or `MockStorage`) to pass concurrency and model checks easily.

### The Reality
Model twins inevitably diverge from production code: they omit error handling, subtle atomic ordering (`Acquire`/`Release` vs `SeqCst`), and ring buffer edge conditions. Bugs hide in the exact details that the twin omitted.

### Strict Verification Rules:
- **Zero-Twin (RFC-0270)**: Route production concurrency primitives through `crate::sync_kernel`. Verification tools (Loom, Stateright) must invoke the **real** production structs and functions.
- **Anti-Vacuity Testing**: Always ask *"Would this proof fail if I injected a bug?"*. All core kernels must pass mutation fuzzing killing $\ge 98\%$ of synthetic AST mutants.
- **Physical DST**: Deterministic Simulation Testing is not complete with `mem_storage=true`. Claims require `PEDRA_SWARM_DISK=1` running on real POSIX filesystems with `pwrite` and `fdatasync` fault injection.
