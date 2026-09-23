-- Theorems over Aeneas extract of dcs lease_kernel.rs (F7/F56).
-- Ord.max.default patched to pass lt, not the Ord instance.
import Aeneas
import LeaseKernel
open Aeneas.Std Result
open pedra_aeneas_lease_kernel

/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

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

/-- Catalog entry (RFC-0218 P2.2, átomo `lease_next_id`): the next
    lease id is exactly the cited chain — saturating_add max_seen 1,
    then clamped below by 1 (Ord.max with the lt instance); ids never
    restart at 1 while a higher id was seen on disk (F7/F56). -/
theorem next_lease_id_after_fate_iff :
    ∀ (max_seen_on_disk : U64) (v : U64),
      (next_lease_id_after max_seen_on_disk = ok v) ↔
        (∃ i : U64,
          lift (core.num.U64.saturating_add max_seen_on_disk 1#u64) = ok i ∧
          core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt i 1#u64
            = ok v) := by
  intro max_seen_on_disk v
  constructor
  · intro h
    unfold next_lease_id_after at h
    obtain ⟨i, hi, h⟩ := bind_ok_inv _ _ _ h
    exact ⟨i, hi, h⟩
  · rintro ⟨i, hi, h⟩
    unfold next_lease_id_after
    exact bind_intro i hi h

/-- Catalog entry (RFC-0218 P2.2, átomo `lease_table`): the table
    verdict is exactly the cited unwrap_or true — a missing entry is
    NOT an expired lease (fail-open only for absence, never for a live
    hit); the AS-IS default flips unknown to false and kills the table
    (F7). -/
theorem lease_table_expired_fate_iff :
    ∀ (table_hit : Option Bool) (v : Bool),
      (lease_table_expired table_hit = ok v) ↔
        (v = core.option.Option.unwrap_or table_hit true) := by
  intro table_hit v
  constructor
  · intro h
    have h' : ok (core.option.Option.unwrap_or table_hit true) = ok v := h
    injection h' with hv
    exact hv.symm
  · intro h
    show ok (core.option.Option.unwrap_or table_hit true) = ok v
    rw [h]
