-- Theorems over Aeneas extract of store apply_kernel.rs (RFC-0152 F10-apply).
import Aeneas
import StoreApplyKernel
open Aeneas.Std Result
open pedra_aeneas_store_apply_kernel

/-- Catalog clone: a log hole stops without advancing. -/
theorem apply_advance_hole_stops :
    apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  unfold apply_advance
  rfl

/-- AS-IS tooth: a hole still applies. -/
theorem apply_advance_as_is_tooth :
    apply_advance_as_is_skip_holes (1#u64) (2#u64) false = ok ApplyAction.Apply := by
  unfold apply_advance_as_is_skip_holes
  rfl

/-- RFC-0191 P2.3 cadence atom: the apply-path Put fate — hist persists
    iff the key is live (not reserved) and the gen is above the gen-0
    floor. Reserved keys never receive applied data at any gen. The
    store trampoline (`apply_range`) matches this. -/
theorem apply_put_plan_hist_iff_live_and_gen_positive :
    ∀ (is_reserved : Bool) (si_gen : U64),
      (apply_put_plan is_reserved si_gen = ok ApplyPutFate.ApplyAndHist)
        ↔ (is_reserved = false ∧ si_gen ≠ 0#u64) := by
  intro is_reserved si_gen
  unfold apply_put_plan
  constructor
  · intro h
    split at h
    · next c1 => exact absurd h (by simp)
    · next c1 =>
      split at h
      · next c2 => exact absurd h (by simp)
      · next c2 => exact ⟨by cases is_reserved <;> simp_all, c2⟩
  · rintro ⟨hf, hz⟩
    split
    · next c1 =>
      rw [c1] at hf
      exact absurd hf (by simp)
    · rfl

/-- AS-IS tooth: the stomp puts applied data even on reserved keys and
    the gen-0 floor. -/
theorem apply_put_plan_as_is_tooth :
    apply_put_plan_as_is true 0#u64 = ok ApplyPutFate.ApplyAndHist := by
  unfold apply_put_plan_as_is
  rfl
