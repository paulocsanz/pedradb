-- Theorems over Aeneas extract of dcs lease_kernel.rs (F7/F56).
-- Ord.max.default patched to pass lt, not the Ord instance.
import Aeneas
import LeaseKernel
open Aeneas.Std Result
open pedra_aeneas_lease_kernel

/-- Catalog entry: lease 0 is immortal. -/
theorem lease_live_zero :
    lease_live (0#u64) (5#u64) = ok true := by
  unfold lease_live
  rfl

/-- AS-IS dente: a past deadline still lives. -/
theorem lease_live_as_is_dente :
    lease_live_as_is (9#u64) (100#u64) = ok true := by
  unfold lease_live_as_is
  rfl

/-- Catalog entry: a lease is live exactly when it is zero (never
    expires) or the clock is still below the expiry (F7/F56 — an
    expired lease is never live). -/
theorem lease_live_iff_zero_or_now_below :
    ∀ (lease : U64) (now_ms : U64),
      (lease_live lease now_ms = ok true)
        ↔ (lease = 0#u64 ∨ now_ms < lease) := by
  intro lease now_ms
  unfold lease_live
  constructor
  · intro h
    split at h
    · next c1 =>
      exact Or.inl c1
    · next c1 =>
      simp at h
      exact Or.inr h
  · rintro (h0 | hlt)
    · rw [if_pos h0]
    · split
      · rfl
      · simp [hlt]
