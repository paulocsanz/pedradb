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

/-! ### RFC-0216 P1.1 4/4 — `form_decode` (átomo `catalog:form_plus`)

O fate do decode como cadeia: combustível = bytes restantes; cada passo
`cont` é exatamente um passo do corpo extraído com índice estritamente
crescente e limitado a `len`; o fim é `i = len` com `out = v`. -/

/-- O `+1#usize` do corpo vale exatamente `↑i + 1` em Nat. -/
private theorem usize_succ_val (i i1 : Usize) (h : (i + 1#usize) = ok i1) :
    (↑i1 : Nat) = (↑i : Nat) + 1 := by
  have he := UScalar.add_equiv i 1#usize
  rw [h] at he
  dsimp only at he
  exact he.2.1

/-- No fim (i = len) o corpo devolve exatamente `done out`. -/
private theorem body_at_end (b : Slice U8) (out : alloc.vec.Vec U8) (i : Usize)
    (hlen : (↑i : Nat) = (b.val).length) :
    form_decode_loop.body b out i = ok (ControlFlow.done out) := by
  have hge : ¬ (i < Slice.len b) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Slice.len b)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold form_decode_loop.body
  dsimp +zeta only
  rw [if_neg hge]

/-- No fim o corpo nunca dá cont. -/
private theorem body_no_cont_at_end (b : Slice U8) (out : alloc.vec.Vec U8)
    (i : Usize) (st : alloc.vec.Vec U8 × Usize)
    (hlen : (↑i : Nat) = (b.val).length)
    (hB : form_decode_loop.body b out i = ok (ControlFlow.cont st)) : False := by
  have hge : ¬ (i < Slice.len b) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Slice.len b)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold form_decode_loop.body at hB
  dsimp +zeta only at hB
  rw [if_neg hge] at hB
  injection hB with hB2
  contradiction

/-- Sob i < len toda folha do corpo é `cont (out', i')` com o índice
estritamente crescente e limitado — a inversão das 11 folhas. -/
private theorem body_inv (b : Slice U8) (out : alloc.vec.Vec U8) (i : Usize)
    (hlt : (↑i : Nat) < (b.val).length)
    (cf : ControlFlow (alloc.vec.Vec U8 × Usize) (alloc.vec.Vec U8))
    (hB : form_decode_loop.body b out i = ok cf) :
    ∃ (out' : alloc.vec.Vec U8) (i' : Usize),
      cf = ControlFlow.cont (out', i') ∧
        (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (b.val).length := by
  have hlt' : i < Slice.len b := by
    refine (UScalar.lt_equiv i (Slice.len b)).mpr ?_
    rw [Aeneas.Std.Slice.len_val]
    exact hlt
  unfold form_decode_loop.body at hB
  dsimp +zeta only at hB
  rw [if_pos hlt'] at hB
  obtain ⟨b1, hb1, hB⟩ := bind_ok_inv _ _ _ hB
  cases b1 with
  | true =>
    obtain ⟨i2, hi2, hB⟩ := bind_ok_inv _ _ _ hB
    split at hB
    · next h43 =>
      obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
      obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
      obtain ⟨i4, hi4, hB⟩ := bind_ok_inv _ _ _ hB
      have hv4 := usize_succ_val i i4 hi4
      refine ⟨out1, i4, (Result.ok.inj hB).symm, by omega, by omega⟩
    · next hn43 =>
      split at hB
      · next h37 =>
        obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
        split at hB
        · next hlt3 =>
          have hn3 : (↑i3 : Nat) < (b.val).length := by
            have h0 := (UScalar.lt_equiv i3 (Slice.len b)).mp hlt3
            rw [Aeneas.Std.Slice.len_val] at h0
            exact h0
          obtain ⟨i5, hi5, hB⟩ := bind_ok_inv _ _ _ hB
          obtain ⟨i6, hi6, hB⟩ := bind_ok_inv _ _ _ hB
          obtain ⟨o, ho, hB⟩ := bind_ok_inv _ _ _ hB
          obtain ⟨i7, hi7, hB⟩ := bind_ok_inv _ _ _ hB
          obtain ⟨o1, ho1, hB⟩ := bind_ok_inv _ _ _ hB
          have hv5 := usize_succ_val i i5 hi5
          cases o with
          | none =>
            obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
            exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
          | some h =>
            cases o1 with
            | none =>
              obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
              exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
            | some l =>
              obtain ⟨i8, hi8, hB⟩ := bind_ok_inv _ _ _ hB
              obtain ⟨i9, hi9, hB⟩ := bind_ok_inv _ _ _ hB
              obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
              obtain ⟨i10, hi10, hB⟩ := bind_ok_inv _ _ _ hB
              have h2 := UScalar.add_equiv i 2#usize
              rw [hi3] at h2
              dsimp only at h2
              have h2v : (↑i3 : Nat) = (↑i : Nat) + 2 := h2.2.1
              have h3 := UScalar.add_equiv i 3#usize
              rw [hi10] at h3
              dsimp only at h3
              have h3v : (↑i10 : Nat) = (↑i : Nat) + 3 := h3.2.1
              exact ⟨out1, i10, (Result.ok.inj hB).symm, by omega, by omega⟩
        · next hge3 =>
          obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
          obtain ⟨i5, hi5, hB⟩ := bind_ok_inv _ _ _ hB
          have hv5 := usize_succ_val i i5 hi5
          exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
      · next hn37 =>
        obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
        have hv3 := usize_succ_val i i3 hi3
        exact ⟨out1, i3, (Result.ok.inj hB).symm, by omega, by omega⟩
  | false =>
    obtain ⟨i2, hi2, hB⟩ := bind_ok_inv _ _ _ hB
    split at hB
    · next h37 =>
      obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
      split at hB
      · next hlt3 =>
        have hn3 : (↑i3 : Nat) < (b.val).length := by
          have h0 := (UScalar.lt_equiv i3 (Slice.len b)).mp hlt3
          rw [Aeneas.Std.Slice.len_val] at h0
          exact h0
        obtain ⟨i5, hi5, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨i6, hi6, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨o, ho, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨i7, hi7, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨o1, ho1, hB⟩ := bind_ok_inv _ _ _ hB
        have hv5 := usize_succ_val i i5 hi5
        cases o with
        | none =>
          obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
          exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
        | some h =>
          cases o1 with
          | none =>
            obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
            exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
          | some l =>
            obtain ⟨i8, hi8, hB⟩ := bind_ok_inv _ _ _ hB
            obtain ⟨i9, hi9, hB⟩ := bind_ok_inv _ _ _ hB
            obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
            obtain ⟨i10, hi10, hB⟩ := bind_ok_inv _ _ _ hB
            have h2 := UScalar.add_equiv i 2#usize
            rw [hi3] at h2
            dsimp only at h2
            have h2v : (↑i3 : Nat) = (↑i : Nat) + 2 := h2.2.1
            have h3 := UScalar.add_equiv i 3#usize
            rw [hi10] at h3
            dsimp only at h3
            have h3v : (↑i10 : Nat) = (↑i : Nat) + 3 := h3.2.1
            exact ⟨out1, i10, (Result.ok.inj hB).symm, by omega, by omega⟩
      · next hge3 =>
        obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
        obtain ⟨i5, hi5, hB⟩ := bind_ok_inv _ _ _ hB
        have hv5 := usize_succ_val i i5 hi5
        exact ⟨out1, i5, (Result.ok.inj hB).symm, by omega, by omega⟩
    · next hn37 =>
      obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
      obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
      have hv3 := usize_succ_val i i3 hi3
      exact ⟨out1, i3, (Result.ok.inj hB).symm, by omega, by omega⟩

/-- Payload de um cont sob i < len progride: i < i' ≤ len. -/
private theorem body_cont_progress (b : Slice U8) (out : alloc.vec.Vec U8)
    (i : Usize) (out' : alloc.vec.Vec U8) (i' : Usize)
    (hlt : (↑i : Nat) < (b.val).length)
    (hB : form_decode_loop.body b out i = ok (ControlFlow.cont (out', i'))) :
    (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (b.val).length := by
  obtain ⟨out2, i2, hcf, hlt2, hle2⟩ :=
    body_inv b out i hlt (ControlFlow.cont (out', i')) hB
  have hp := ControlFlow.cont.inj hcf
  obtain ⟨-, hii⟩ := Prod.mk.inj hp
  subst hii
  exact ⟨hlt2, hle2⟩

/-- Done só no fim, com o out intacto. -/
private theorem body_done_end (b : Slice U8) (out : alloc.vec.Vec U8)
    (i : Usize) (r : alloc.vec.Vec U8)
    (hle : (↑i : Nat) ≤ (b.val).length)
    (hB : form_decode_loop.body b out i = ok (ControlFlow.done r)) :
    (↑i : Nat) = (b.val).length ∧ out = r := by
  by_cases hlt : (↑i : Nat) < (b.val).length
  · obtain ⟨out2, i2, hcf, -, -⟩ :=
      body_inv b out i hlt (ControlFlow.done r) hB
    exact absurd hcf (by intro hh; contradiction)
  · have hlen : (↑i : Nat) = (b.val).length := by omega
    refine ⟨hlen, ?_⟩
    have h := (body_at_end b out i hlen).symm.trans hB
    exact ControlFlow.done.inj (Result.ok.inj h)

/-- O fate do decode como cadeia: combustível = bytes restantes; cada
passo `cont` consume pelo menos um byte (i' > i, i' ≤ len) e o fim é
o `done` exato em i = len com out = v. -/
private def DecodeFate (b : Slice U8) :
    Nat → alloc.vec.Vec U8 → Usize → alloc.vec.Vec U8 → Prop
  | 0, out, i, v =>
      (↑i : Nat) = (b.val).length ∧ out = v
  | fuel + 1, out, i, v =>
      (∃ (out' : alloc.vec.Vec U8) (i' : Usize),
          form_decode_loop.body b out i = ok (ControlFlow.cont (out', i')) ∧
            DecodeFate b fuel out' i' v) ∨
        ((↑i : Nat) = (b.val).length ∧ out = v)

/-- O fate do loop por indução no combustível. -/
private theorem form_decode_loop_fate (b : Slice U8) :
    ∀ (fuel : Nat) (out : alloc.vec.Vec U8) (i : Usize),
      (↑i : Nat) ≤ (b.val).length → (b.val).length - (↑i : Nat) ≤ fuel →
      ∀ v : alloc.vec.Vec U8,
        (form_decode_loop b out i = ok v) ↔ DecodeFate b fuel out i v := by
  intro fuel
  induction fuel with
  | zero =>
    intro out i hile hfuel v
    have hlen : (↑i : Nat) = (b.val).length := by omega
    constructor
    · intro h
      refine ⟨hlen, ?_⟩
      unfold form_decode_loop at h
      rw [loop.eq_def] at h
      dsimp only at h
      cases hB : form_decode_loop.body b out i with
      | ok cf =>
        cases cf with
        | done r =>
          rw [hB] at h
          dsimp only at h
          rw [Result.ok.inj h] at hB
          exact (body_done_end b out i v hile hB).2
        | cont st =>
          exact absurd hB (body_no_cont_at_end b out i st hlen)
      | fail e =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
      | div =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
    · rintro ⟨-, hout⟩
      unfold form_decode_loop
      rw [loop.eq_def]
      dsimp only
      rw [body_at_end b out i hlen, hout]
  | succ fuel ih =>
    intro out i hile hfuel v
    unfold form_decode_loop
    rw [loop.eq_def]
    dsimp only
    cases hB : form_decode_loop.body b out i with
    | ok cf =>
      cases cf with
      | cont st =>
        obtain ⟨out', i'⟩ := st
        dsimp only
        by_cases hlt : (↑i : Nat) < (b.val).length
        · obtain ⟨hprog1, hprog2⟩ := body_cont_progress b out i out' i' hlt hB
          constructor
          · intro h
            exact Or.inl ⟨out', i', hB, (ih out' i' hprog2 (by omega) v).mp h⟩
          · rintro (⟨out2, i2, hbody, hfate⟩ | ⟨hlen, hout⟩)
            · have hu : ControlFlow.cont (out', i')
                  = ControlFlow.cont (out2, i2) :=
                  Result.ok.inj (hB.symm.trans hbody)
              obtain ⟨hoo, hii⟩ := Prod.mk.inj (ControlFlow.cont.inj hu)
              subst hoo
              subst hii
              exact (ih out' i' hprog2 (by omega) v).mpr hfate
            · exact absurd hlt (by omega)
        · have hlen : (↑i : Nat) = (b.val).length := by omega
          exact absurd hB (body_no_cont_at_end b out i (out', i') hlen)
      | done r =>
        dsimp only
        obtain ⟨hlen, hout⟩ := body_done_end b out i r hile hB
        constructor
        · intro h
          have hrv : r = v := Result.ok.inj h
          exact Or.inr ⟨hlen, hrv ▸ hout⟩
        · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen2, hout2⟩)
          · have hne := hbody.symm.trans hB
            injection hne with hne2
            contradiction
          · exact congrArg ok (hout.symm.trans hout2)
    | fail e =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen, hout⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((body_at_end b out i hlen).symm.trans hB) (by simp)
    | div =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen, hout⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((body_at_end b out i hlen).symm.trans hB) (by simp)

/-- RFC-0216 P1.1 4/4 (átomo `catalog:form_plus`, entrada
`form_decode`): o output inteiro do decoder é exatamente a cadeia
citada dos passos do corpo extraído — cada byte consumido passa por
`plus_before_percent`/`43→32`/`37→from_hex×2`, o `+3` só ocorre com
dois hex válidos sob guarda `i+2 < len`, e o resultado final é o out
acumulado exatamente em `i = len`. O mutante AS-IS troca o ramo do
`+`; planta `form_decode_on_live_http_is_not_ok` recusa no handler
vivo. -/
theorem form_decode_fate_iff :
    ∀ (s : Str) (b : Slice U8) (v : alloc.vec.Vec U8)
      (hb : core.str.Str.as_bytes s = ok b),
      (form_decode s = ok v) ↔
        DecodeFate b (b.val).length
          (alloc.vec.Vec.with_capacity U8 (Slice.len b)) 0#usize v := by
  intro s b v hb
  have hloop : form_decode s
      = form_decode_loop b (alloc.vec.Vec.with_capacity U8 (Slice.len b))
          0#usize := by
    unfold form_decode
    rw [hb]
    simp only [Aeneas.Std.bind_tc_ok]
  rw [hloop]
  exact form_decode_loop_fate b (b.val).length _ 0#usize (Nat.zero_le _)
    (Nat.sub_le _ _) v


/-! ### RFC-0216 P1.2 — query ×3 átomo -/

/-- RFC-0216 P1.2 1/3 (átomo `catalog:query_u64_conflict`): o conflito
u64 é exatamente a desigualdade decidida dos dois lados; o AS-IS
sempre responde "sem conflito". -/
theorem query_u64_conflict_fate_iff :
    ∀ (a b : U64) (r : Bool),
      (query_u64_conflict a b = ok r) ↔ r = (a != b) := by
  intro a b r
  constructor
  · intro hval
    unfold query_u64_conflict at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold query_u64_conflict
    rw [hr]

/-- RFC-0216 P1.2 2/3 (átomo `catalog:query_part_is_bare_name`): o
veredito "parte é nome puro" é exatamente a cadeia citada — parte
vazia ⇒ falso, parte com `=` ⇒ falso, senão o decode da parte
comparado byte a byte com a chave via o eq extraído. -/
theorem query_part_is_bare_name_fate_iff :
    ∀ (part key : Str) (r : Bool),
      (query_part_is_bare_name part key = ok r) ↔
        ((core.str.Str.is_empty part = ok true ∧ r = false) ∨
          (core.str.Str.is_empty part = ok false ∧
            ((core.str.Str.contains part '=' = ok true ∧ r = false) ∨
              (core.str.Str.contains part '=' = ok false ∧
                (∃ (v : alloc.vec.Vec U8) (s : Slice U8),
                    form_decode part = ok v ∧
                      core.str.Str.as_bytes key = ok s ∧
                        alloc.vec.Vec.Insts.CoreCmpPartialEqShared0Slice.eq Global
                          core.cmp.PartialEqU8 v s = ok r))))) := by
  intro part key r
  constructor
  · intro hval
    unfold query_part_is_bare_name at hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hbt =>
      exact Or.inl ⟨by rw [← hbt]; exact hb, (Result.ok.inj hval).symm⟩
    · next hbf =>
      have hbf' : b = false := by simp at hbf; exact hbf
      obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
      refine Or.inr ⟨by rw [← hbf']; exact hb, ?_⟩
      split at hval
      · next hbt1 =>
        exact Or.inl ⟨by rw [← hbt1]; exact hb1, (Result.ok.inj hval).symm⟩
      · next hbf1 =>
        have hbf1' : b1 = false := by simp at hbf1; exact hbf1
        obtain ⟨v, hv, hval⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨s, hs, hval⟩ := bind_ok_inv _ _ _ hval
        exact Or.inr ⟨by rw [← hbf1']; exact hb1, v, s, hv, hs, hval⟩
  · rintro (⟨hb, rfl⟩ | ⟨hb, (⟨hb1, rfl⟩ | ⟨hb1, v, s, hv, hs, hval⟩)⟩)
    · unfold query_part_is_bare_name
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold query_part_is_bare_name
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [if_neg (by simp), hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      simp
    · unfold query_part_is_bare_name
      rw [hb]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [if_neg (by simp), hb1]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [if_neg (by simp)]
      rw [hv]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [hs]
      simp only [Aeneas.Std.bind_tc_ok]
      exact hval

