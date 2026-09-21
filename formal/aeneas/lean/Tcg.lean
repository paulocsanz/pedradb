-- Theorems over Aeneas extract of tcg.rs
import Aeneas
import TcgKernel
open Aeneas.Std Result
open pedra_aeneas_tcg_kernel

theorem tcg_guest_admitted_true :
    tcg_guest_admitted true = ok true := by
  unfold tcg_guest_admitted
  rfl

/-- Native World passes `guest_reachable = false`; the kernel is that flag. -/
theorem tcg_guest_admitted_fate_iff :
    ∀ (guest v : Bool),
      (tcg_guest_admitted guest = ok v) ↔ (v = guest) := by
  intro guest v
  unfold tcg_guest_admitted
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl
