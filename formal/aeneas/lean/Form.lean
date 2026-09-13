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

/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

theorem from_hex_fate_iff :
    ∀ (c : Aeneas.Std.U8) (r : Option Aeneas.Std.U8),
      (from_hex c = ok r) ↔
        ((48#u8 ≤ c ∧ ((c ≤ 57#u8 ∧ ∃ i, c - 48#u8 = ok i ∧ r = some i) ∨ (¬(c ≤ 57#u8) ∧ ((97#u8 ≤ c ∧ ((c ≤ 102#u8 ∧ ∃ i i1, c - 97#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 102#u8) ∧ ((65#u8 ≤ c ∧ ((c ≤ 70#u8 ∧ ∃ i i1, c - 65#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 70#u8) ∧ r = none))) ∨ (¬(65#u8 ≤ c) ∧ r = none))))) ∨ (¬(97#u8 ≤ c) ∧ ((65#u8 ≤ c ∧ ((c ≤ 70#u8 ∧ ∃ i i1, c - 65#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 70#u8) ∧ r = none))) ∨ (¬(65#u8 ≤ c) ∧ r = none))))))) ∨ (¬(48#u8 ≤ c) ∧ ((97#u8 ≤ c ∧ ((c ≤ 102#u8 ∧ ∃ i i1, c - 97#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 102#u8) ∧ ((65#u8 ≤ c ∧ ((c ≤ 70#u8 ∧ ∃ i i1, c - 65#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 70#u8) ∧ r = none))) ∨ (¬(65#u8 ≤ c) ∧ r = none))))) ∨ (¬(97#u8 ≤ c) ∧ ((65#u8 ≤ c ∧ ((c ≤ 70#u8 ∧ ∃ i i1, c - 65#u8 = ok i ∧ i + 10#u8 = ok i1 ∧ r = some i1) ∨ (¬(c ≤ 70#u8) ∧ r = none))) ∨ (¬(65#u8 ≤ c) ∧ r = none)))))) := by
  intro c r
  constructor
  · intro hval
    unfold from_hex at hval
    split at hval
    · next h48 =>
        split at hval
        · next h57 =>
            obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
            exact Or.inl ⟨h48, Or.inl ⟨h57, i, hi, (Result.ok.inj hval).symm⟩⟩
        · next hn57 =>
            split at hval
            · next h97 =>
                split at hval
                · next h102 =>
                    obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                    obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                    exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inl ⟨h97, Or.inl ⟨h102, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩⟩
                · next hn102 =>
                    split at hval
                    · next h65 =>
                        split at hval
                        · next h70 =>
                            obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                            obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                            exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inl ⟨h65, Or.inl ⟨h70, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩⟩
                        · next hn70 =>
                            exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inl ⟨h65, Or.inr ⟨hn70, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩⟩
                    · next hn65 =>
                        exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inr ⟨hn65, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩
            · next hn97 =>
                split at hval
                · next h65 =>
                    split at hval
                    · next h70 =>
                        obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                        obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                        exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inr ⟨hn97, Or.inl ⟨h65, Or.inl ⟨h70, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩
                    · next hn70 =>
                        exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inr ⟨hn97, Or.inl ⟨h65, Or.inr ⟨hn70, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩
                · next hn65 =>
                    exact Or.inl ⟨h48, Or.inr ⟨hn57, Or.inr ⟨hn97, Or.inr ⟨hn65, (Result.ok.inj hval).symm⟩⟩⟩⟩
    · next hn48 =>
        split at hval
        · next h97 =>
            split at hval
            · next h102 =>
                obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                exact Or.inr ⟨hn48, Or.inl ⟨h97, Or.inl ⟨h102, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩
            · next hn102 =>
                    split at hval
                    · next h65 =>
                        split at hval
                        · next h70 =>
                            obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                            obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                            exact Or.inr ⟨hn48, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inl ⟨h65, Or.inl ⟨h70, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩
                        · next hn70 =>
                            exact Or.inr ⟨hn48, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inl ⟨h65, Or.inr ⟨hn70, (Result.ok.inj hval).symm⟩⟩⟩⟩⟩
                    · next hn65 =>
                        exact Or.inr ⟨hn48, Or.inl ⟨h97, Or.inr ⟨hn102, Or.inr ⟨hn65, (Result.ok.inj hval).symm⟩⟩⟩⟩
        · next hn97 =>
                split at hval
                · next h65 =>
                    split at hval
                    · next h70 =>
                        obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
                        obtain ⟨i1, hi1, hval⟩ := bind_ok_inv _ _ _ hval
                        exact Or.inr ⟨hn48, Or.inr ⟨hn97, Or.inl ⟨h65, Or.inl ⟨h70, i, i1, hi, hi1, (Result.ok.inj hval).symm⟩⟩⟩⟩
                    · next hn70 =>
                        exact Or.inr ⟨hn48, Or.inr ⟨hn97, Or.inl ⟨h65, Or.inr ⟨hn70, (Result.ok.inj hval).symm⟩⟩⟩⟩
                · next hn65 =>
                    exact Or.inr ⟨hn48, Or.inr ⟨hn97, Or.inr ⟨hn65, (Result.ok.inj hval).symm⟩⟩⟩
  · rintro (⟨h48, (⟨h57, i, hi, hr⟩ | ⟨hn57, (⟨h97, (⟨h102, i, i1, hi, hi1, hr⟩ | ⟨hn102, (⟨h65, (⟨h70, i, i1, hi, hi1, hr⟩ | ⟨hn70, hr⟩)⟩ | ⟨hn65, hr⟩)⟩)⟩ | ⟨hn97, (⟨h65, (⟨h70, i, i1, hi, hi1, hr⟩ | ⟨hn70, hr⟩)⟩ | ⟨hn65, hr⟩)⟩)⟩)⟩ | ⟨hn48, (⟨h97, (⟨h102, i, i1, hi, hi1, hr⟩ | ⟨hn102, (⟨h65, (⟨h70, i, i1, hi, hi1, hr⟩ | ⟨hn70, hr⟩)⟩ | ⟨hn65, hr⟩)⟩)⟩ | ⟨hn97, (⟨h65, (⟨h70, i, i1, hi, hi1, hr⟩ | ⟨hn70, hr⟩)⟩ | ⟨hn65, hr⟩)⟩)⟩)
    · unfold from_hex
      rw [if_pos h48, if_pos h57, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_pos h97, if_pos h102, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_pos h97, if_neg hn102, if_pos h65, if_pos h70, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_pos h97, if_neg hn102, if_pos h65, if_neg hn70]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_pos h97, if_neg hn102, if_neg hn65]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_neg hn97, if_pos h65, if_pos h70, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_neg hn97, if_pos h65, if_neg hn70]
      rw [hr]
    · unfold from_hex
      rw [if_pos h48, if_neg hn57, if_neg hn97, if_neg hn65]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_pos h97, if_pos h102, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_pos h97, if_neg hn102, if_pos h65, if_pos h70, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_pos h97, if_neg hn102, if_pos h65, if_neg hn70]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_pos h97, if_neg hn102, if_neg hn65]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_neg hn97, if_pos h65, if_pos h70, hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hi1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_neg hn97, if_pos h65, if_neg hn70]
      rw [hr]
    · unfold from_hex
      rw [if_neg hn48, if_neg hn97, if_neg hn65]
      rw [hr]
