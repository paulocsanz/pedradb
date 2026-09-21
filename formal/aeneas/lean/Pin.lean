-- Theorems over Aeneas extract of pin_kernel.rs
import Aeneas
import PinKernel
open Aeneas.Std Result
open pedra_aeneas_pin_kernel

theorem may_advance_pin_forward :
    may_advance_pin 3#u64 5#u64 = ok true := by
  unfold may_advance_pin
  have h : (5#u64 > 3#u64) = true := by native_decide
  simp [h]

/-- RFC-0218 P2.1 1/12 (atom `catalog:journal_catch_up_pin`, entrada
    `catch_up_pins_on_read`): ler com pins atrasados SEMPRE alcança os
    pins — EXATAMENTE a constante citada true. O AS-IS é false (pins
    ficam para trás na leitura — tooth plantado). -/
theorem catch_up_pins_on_read_fate_iff :
    ∀ (v : Bool),
      (catch_up_pins_on_read = ok v) ↔ (v = true) := by
  intro v
  constructor
  · intro hval
    unfold catch_up_pins_on_read at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P2.1 2/12 (atom `catalog:journal_fold_pin`, entrada
    `fold_pins_on_read`): varrer NUNCA segura pins — EXATAMENTE a
    constante citada false. O AS-IS é true (o fold congela o journal
    — tooth plantado). -/
theorem fold_pins_on_read_fate_iff :
    ∀ (v : Bool),
      (fold_pins_on_read = ok v) ↔ (v = false) := by
  intro v
  constructor
  · intro hval
    unfold fold_pins_on_read at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P2.1 3/12 (atom `catalog:journal_pin`, entrada
    `may_advance_pin`): avançar o pin é EXATAMENTE o lift citado
    `applied_through > pin` — o journal só solta o que já foi
    aplicado. O AS-IS nunca segura (pin anda antes do applied —
    tooth plantado). -/
theorem may_advance_pin_fate_iff :
    ∀ (pin : U64) (applied_through : U64) (v : Bool),
      (may_advance_pin pin applied_through = ok v) ↔
      (v = core.cmp.impls.PartialOrdU64.gt applied_through pin) := by
  intro pin applied_through v
  constructor
  · intro hval
    unfold may_advance_pin at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P2.1 4/12 (atom `catalog:journal_next_pin`, entrada
    `next_pin`): o próximo pin é EXATAMENTE o citado — sem batch, o
    pin fica; com batch_max, o pin anda para o batch_max somente
    quando ele está à frente. O AS-IS anda para trás (pin pode
    regredir — tooth plantado). -/
theorem next_pin_fate_iff :
    ∀ (pin : U64) (batch_max : Option U64) (r : U64),
      (next_pin pin batch_max = ok r) ↔
        ((batch_max = none ∧ r = pin) ∨
         (∃ m : U64, batch_max = some m ∧
           ((m > pin ∧ r = m) ∨ (¬ (m > pin) ∧ r = pin)))) := by
  intro pin batch_max r
  constructor
  · intro hval
    unfold next_pin at hval
    cases batch_max with
    | none =>
      dsimp only at hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    | some m =>
      dsimp only at hval
      refine Or.inr ⟨m, rfl, ?_⟩
      split at hval
      · next hm =>
        injection hval with hv
        exact Or.inl ⟨hm, hv.symm⟩
      · next hm =>
        injection hval with hv
        exact Or.inr ⟨hm, hv.symm⟩
  · rintro (⟨rfl, hv⟩ | ⟨m, rfl, (⟨hm, hv⟩ | ⟨hm, hv⟩)⟩)
    · subst hv
      unfold next_pin
      rfl
    · subst hv
      unfold next_pin
      dsimp only
      rw [if_pos hm]
    · subst hv
      unfold next_pin
      dsimp only
      rw [if_neg (by simp [hm])]
