# Wait-for cycle is deadlock (MachCSL + static Rust conflict-lock)

**Date:** 2026-09-07
**Primary sources:**
- Kaashoek, Zeldovich, *Extending concurrent separation logic to the hardware level to verify the xv6 OS kernel on RISC-V with AI agents*, arXiv:2609.04043 (3 Sep 2026). PDF: `2609.04043.pdf`. MachCSL adapts Iris CSL to RISC-V sub-instruction concurrency, including fine-grained locking; the xv6 case study found lock-related bugs.
- Li et al., *Static Deadlock Detection for Rust Programs*, arXiv:2401.01114. PDF: `2401.01114.pdf`. Conflict lock: two threads acquire locks in opposing order — a wait-for cycle.

## What the sources actually say

MachCSL is not a Pedra proof; it is Iris-based CSL at hardware grain. The relevant fact: lock-order / wait-for is a named concurrency theorem, not “parking_lot is the TCB so skip deadlock.” Static detection treats a cycle in the wait-for graph as deadlock (conflict lock); missing the cycle is a distinct bug class from double-lock.

## Used this turn

Catalog pair `wait_for_deadlock` (`data_fate`): 2PL `LockTable::lock` calls production `wait_for_deadlock` when `detect`. Cycle ⇒ true (Deadlock); AS-IS always false. Production `locktab.rs` is now the Verus term (`single_artifact`); Verus proves the two-cycle stand-in (`lemma_two_cycle_is_deadlock`); rustc still walks `HashMap`/`HashSet`. `LockTable` Condvar stays rustc-only.

## Not claimed

Iris proof of `parking_lot`. Dump of `LockTable::lock`. ∀π scheduler. “somos seL4”. MachCSL of Pedra.
