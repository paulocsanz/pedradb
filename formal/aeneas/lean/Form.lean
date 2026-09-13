-- Theorems over Aeneas extract of form_kernel.rs (form-urlencoded query).
-- Charon --exclude str::contains / pattern; contains Pattern hole and
-- query_values_conflict Iterator.any patched in aeneas_form.sh.
import Aeneas
import FormKernel
open Aeneas.Std Result
open pedra_aeneas_form_kernel

/-- Catalog entry: `+` maps to space. -/
theorem form_plus_byte_plus :
    form_plus_byte (43#u8) = ok (32#u8) := by
  unfold form_plus_byte
  rfl

/-- AS-IS dente: `+` stays `+`. -/
theorem form_plus_byte_as_is_dente :
    form_plus_byte_as_is (43#u8) = ok (43#u8) := by
  unfold form_plus_byte_as_is
  rfl

/-- Catalog entry: plus is mapped before percent. -/
theorem plus_before_percent_true :
    plus_before_percent = ok true := by
  unfold plus_before_percent
  rfl

/-- Catalog entry: parsed query ints disagree. -/
theorem query_u64_conflict_diff :
    query_u64_conflict (1#u64) (0#u64) = ok true := by
  unfold query_u64_conflict
  rfl

/-- AS-IS dente: last/first wins, never reject. -/
theorem query_u64_conflict_as_is_dente :
    query_u64_conflict_as_is (1#u64) (0#u64) = ok false := by
  unfold query_u64_conflict_as_is
  rfl

theorem form_plus_byte_fate_iff :
    ∀ (b r : Aeneas.Std.U8),
      (form_plus_byte b = ok r) ↔
        ((b = 43#u8 ∧ r = 32#u8) ∨ (¬(b = 43#u8) ∧ r = b)) := by
  intro b r
  constructor
  · intro hval
    unfold form_plus_byte at hval
    split at hval
    · next hbt => exact Or.inl ⟨hbt, (Result.ok.inj hval).symm⟩
    · next hbf => exact Or.inr ⟨hbf, (Result.ok.inj hval).symm⟩
  · rintro (⟨hbt, hr⟩ | ⟨hbf, hr⟩)
    · unfold form_plus_byte
      rw [if_pos hbt, hr]
    · unfold form_plus_byte
      rw [if_neg hbf, hr]

theorem plus_before_percent_fate_iff :
    ∀ (r : Bool), (plus_before_percent = ok r) ↔ r = true := by
  intro r
  constructor
  · intro hval
    unfold plus_before_percent at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold plus_before_percent
    rw [hr]
