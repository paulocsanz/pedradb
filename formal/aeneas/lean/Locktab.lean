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

/-! ### Step bridges (RFC-0202 P1.1, fila deadlock)
    Cada aresta de saída de UM passo do detector, com hipóteses de
    lookup: o corpo extraído chama HashMap/HashSet como axiomas; as
    hipóteses fixam o resultado dessas chamadas e o corpo reduz por
    defeq. O iff completo do ciclo (semântica de mapa) fica TCB —
    fronteira datada em `formal/aeneas/EXTRACT.md`. -/

/-- Step edge: fresh owner that waits for NOBODY is reported alive
    (`done false`) — the detector never invents a cycle without a
    wait-for edge. -/
theorem wait_for_deadlock_step_nowait_is_alive :
    ∀ (owned waiting waiter owner seen seen'),
      std.collections.hash.set.HashSet.insert core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          seen owner = ok (true, seen') →
      std.collections.hash.map.HashMap.get core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          (core.borrow.Borrow.Blanket Aeneas.Std.U64) U64.Insts.CoreHashHash
          core.cmp.EqU64 waiting owner = ok none →
      wait_for_deadlock_loop.body owned waiting waiter owner seen
        = ok (ControlFlow.done false) := by
  intro owned waiting waiter owner seen seen' hseen hnowait
  unfold wait_for_deadlock_loop.body
  rw [hseen, hnowait]
  simp

/-- Step edge: fresh owner waiting on a key OWNED by the waiter itself
    closes the cycle — the detector reports deadlock (`done true`). -/
theorem wait_for_deadlock_step_cycle_closes :
    ∀ (owned waiting waiter owner seen seen' k),
      std.collections.hash.set.HashSet.insert core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          seen owner = ok (true, seen') →
      std.collections.hash.map.HashMap.get core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          (core.borrow.Borrow.Blanket Aeneas.Std.U64) U64.Insts.CoreHashHash
          core.cmp.EqU64 waiting owner = ok (some k) →
      std.collections.hash.map.HashMap.get bytes.bytes.Bytes.Insts.CoreCmpEq
          bytes.bytes.Bytes.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          (core.borrow.Borrow.Blanket bytes.bytes.Bytes)
          bytes.bytes.Bytes.Insts.CoreHashHash
          bytes.bytes.Bytes.Insts.CoreCmpEq owned k = ok (some waiter) →
      wait_for_deadlock_loop.body owned waiting waiter owner seen
        = ok (ControlFlow.done true) := by
  intro owned waiting waiter owner seen seen' k hseen hwait hkey
  unfold wait_for_deadlock_loop.body
  rw [hseen, hwait]
  simp [hkey]

/-- Step edge: owner already in `seen` (revisit) is reported as a cycle
    (`done true`) — the detector terminates on revisit, never loops. -/
theorem wait_for_deadlock_step_revisit_reports_cycle :
    ∀ (owned waiting waiter owner seen seen'),
      std.collections.hash.set.HashSet.insert core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          seen owner = ok (false, seen') →
      wait_for_deadlock_loop.body owned waiting waiter owner seen
        = ok (ControlFlow.done true) := by
  intro owned waiting waiter owner seen seen' hseen
  unfold wait_for_deadlock_loop.body
  rw [hseen]
  simp

