# RFC-0270: Zero-Twin Verification Policy (The Anti-Vacuity Protocol)

## 1. The Problem: The "Twin Code" Gap (O Problema do Gêmeo)
Historically, formal verification in PedraDB (and many other systems) has relied on writing abstracted "models" or "twins" of the production code. 
- A `LoomWriteGroup` mock struct to test `WriteGroup` logic.
- A Stateright model that implements a simplified state machine rather than calling the real `.submit()` and `wal_ticket_kernel`.
- Lean 4 functional pure extracts that omit `parking_lot` Mutexes and atomics.

**The Danger:** If the verification model and the production AST drift by a single line (e.g., an inverted `if`, a dropped `Acquire` ordering, a missed lock release), the proof passes 100% green while the production system crashes. This is the **Model-Implementation Semantic Gap**. It makes our verification vacuous.

## 2. The Policy: Zero Twins

From this RFC forward, **no verification artifact may verify a re-implemented model**. Verification must execute or analyze the production Abstract Syntax Tree (AST) directly.

### 2.1 Loom: The `sync_kernel` Shim
Production concurrent code MUST NOT use `std::sync` or `parking_lot` directly. It must use `crate::sync_kernel::*`.
- Under `#[cfg(not(loom))]`, this routes to `std`/`parking_lot` (zero-cost).
- Under `#[cfg(loom)]`, this routes to `loom::sync` and `loom::thread`.
- **Rule:** `loom::model(|| ...)` tests MUST instantiate the *real* production struct (e.g., `WriteGroup::new()`) and call its *real* methods. 

### 2.2 Stateright: Production Hooks
Stateright models (`tests/*_model.rs`) may abstract the *input generation* (the permutations of clients and inputs), but the state transition step `next_state` MUST invoke the actual production `fn`s (e.g., `write_admission_kernel::*` or `bloom::*`) to compute the next state. Re-implementing the logic in the model is forbidden.

### 2.3 Verus: Ghost State vs `vstd`
Verus rejects many standard Rust types. To verify production code with Verus without rewriting it into `vstd` types:
1. Use `verus_keep_ghost` and `proof` blocks to weave mathematical assertions into the mainline code.
2. For algorithms that MUST be proven purely, isolate them into strict `no_std` pure-functional kernels (e.g., `lsm_r1_kernel.rs`), which are invoked by the dirty I/O layer. The pure kernel is verified; the dirty layer is tested via DST.

### 2.4 Spec Anti-Vacuity (Mutation Testing)
Every verified property must have at least one documented "mutant" (a deliberately injected bug, like `seq + 2` instead of `seq + 1`) that causes the proof tool to fail. If a mutant passes the verification suite, the verification is declared vácua (vacuous) and rejected.
