-- Theorems over Aeneas extract of locktab.rs wait_for_deadlock (RFC-0150 P2c).
-- Charon --start-from wait_for_deadlock; --exclude LockTable (nested borrows).
import Aeneas
import LocktabKernel
open Aeneas.Std Result
open pedra_aeneas_locktab_kernel

/-- Catalog entry: extracted wait-for is new HashSet then the loop. -/
theorem wait_for_deadlock_is_loop
    (owned waiting waiter owner) :
    wait_for_deadlock owned waiting waiter owner = (
      do
        let seen ← std.collections.hash.set.HashSetTRandomStateGlobal.new U64
        wait_for_deadlock_loop owned waiting waiter owner seen
    ) := by
  unfold wait_for_deadlock
  rfl

/-- AS-IS dente: the cycle is never a deadlock. -/
theorem wait_for_deadlock_as_is_dente
    (owned waiting waiter owner) :
    wait_for_deadlock_as_is owned waiting waiter owner = ok false := by
  unfold wait_for_deadlock_as_is
  rfl

/-- Dual-unfold: production wait-for is new-set then the extracted loop;
    as-is on the same arguments never reports a cycle (HashMap stays axiom). -/
theorem wait_for_deadlock_loop_vs_as_is
    (owned waiting waiter owner) :
    wait_for_deadlock owned waiting waiter owner = (
      do
        let seen ← std.collections.hash.set.HashSetTRandomStateGlobal.new U64
        wait_for_deadlock_loop owned waiting waiter owner seen
    ) ∧ wait_for_deadlock_as_is owned waiting waiter owner = ok false := by
  constructor
  · unfold wait_for_deadlock; rfl
  · unfold wait_for_deadlock_as_is; rfl
