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

/-- RFC-0218 P2.1 1/12 (átomo `catalog:journal_catch_up_pin`, entrada
    `catch_up_pins_on_read`): ler com pins atrasados SEMPRE alcança os
    pins — EXATAMENTE a constante citada true. O AS-IS é false (pins
    ficam para trás na leitura — dente plantado). -/
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

/-- RFC-0218 P2.1 2/12 (átomo `catalog:journal_fold_pin`, entrada
    `fold_pins_on_read`): varrer NUNCA segura pins — EXATAMENTE a
    constante citada false. O AS-IS é true (o fold congela o journal
    — dente plantado). -/
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
