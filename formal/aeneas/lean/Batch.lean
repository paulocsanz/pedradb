-- Theorems over Aeneas extract of batch.rs (RFC-0150 P2a).
-- Charon --start-from write_record_count_ok (decode has early-return-in-loop).
import Aeneas
import BatchKernel
open Aeneas.Std Result
open pedra_aeneas_batch_kernel

/-- Catalog entry: prefix count is not Ok. -/
theorem write_record_count_ok_prefix :
    batch.write_record_count_ok (3#u32) (2#usize) = ok false := by
  unfold batch.write_record_count_ok
  have h : UScalar.cast UScalarTy.Usize (3#u32) = 3#usize := by native_decide
  simp [h, lift]

/-- AS-IS tooth: prefix still admits. -/
theorem write_record_count_ok_as_is_tooth :
    batch.write_record_count_ok_as_is (3#u32) (2#usize) = ok true := by
  unfold batch.write_record_count_ok_as_is
  rfl

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

/-- RFC-0213 P1.2 (storage cadence, atom `catalog:write_record_count`):
    a decoded batch length matches its prefix count EXACTLY along the
    extracted route — the count is cast u32→usize (the honest lift)
    and the comparison decides, with the cast step ok (fate forall
    over the extracted body, RFC-0170 P2.4). The AS-IS mutant admits
    any length (the torn-batch lie the DST plant
    `write_record_count_ok_on_live_torn_batch_is_not_ok` refutes). -/
theorem write_record_count_ok_fate_iff :
    ∀ (count : U32) (decoded_len : Usize) (v : Bool),
    (batch.write_record_count_ok count decoded_len = ok v) ↔
      (∃ i, lift (UScalar.cast UScalarTy.Usize count) = ok i ∧
        v = decide (decoded_len = i)) := by
  intro count decoded_len v
  unfold batch.write_record_count_ok
  constructor
  · intro hval
    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
    exact ⟨i, hi, by injection hval with hv; exact hv.symm⟩
  · rintro ⟨i, hi, hv⟩
    refine bind_intro i hi ?_
    exact congrArg ok hv.symm
