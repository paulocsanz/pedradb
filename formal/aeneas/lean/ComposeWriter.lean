-- RFC-0222 P2.1 / RFC-0220 P0.2 — writer-spine dual-unfold:
-- flusher_gate_plan × parked_debt_plan.
--
-- Sem worker (Workerless) NADA parqueia — inclusive com dívida no cap.
-- O AS-IS diz WorkerDrains sempre (writer workerless dorme para sempre).
-- Cada perna é o átomo iff já registrado; os corpos extraídos não abrem.
import Aeneas
import Flush
open Aeneas.Std Result
open pedra_aeneas_flush_kernel

/-- RFC-0222 P2.1: a cadeia workerless COMPOSTA — o gate é Workerless
    EXATAMENTE quando nenhum worker está attached, e a dívida parked é
    DebtAtCap EXATAMENTE no/acima do cap. Dual-unfold dos dois átomos
    (`catalog:flusher_gate_plan`, `catalog:parked_debt_plan`). Sem worker
    o plano NÃO vira WorkerDrains mesmo com dívida no cap. -/
theorem writer_workerless_gate_and_debt_iff :
    ∀ (attached : Bool) (parked cap : U64)
      (g : FlusherGate) (d : ParkedDebtPlan),
      (flusher_gate_plan attached = ok g ∧
          parked_debt_plan parked cap = ok d) ↔
        (((attached = true ∧ g = FlusherGate.WorkerDrains) ∨
            (attached = false ∧ g = FlusherGate.Workerless)) ∧
          ((parked < cap ∧ d = ParkedDebtPlan.NoDebtBelowCap) ∨
            (¬(parked < cap) ∧ d = ParkedDebtPlan.DebtAtCap))) := by
  intro attached parked cap g d
  constructor
  · intro ⟨hg, hd⟩
    exact ⟨(flusher_gate_plan_fate_iff attached g).mp hg,
      (parked_debt_plan_fate_iff parked cap d).mp hd⟩
  · intro ⟨hg, hd⟩
    exact ⟨(flusher_gate_plan_fate_iff attached g).mpr hg,
      (parked_debt_plan_fate_iff parked cap d).mpr hd⟩

/-- Sem worker, o gate composto nunca é WorkerDrains — mesmo com
    parked ≥ cap (a dívida existe, mas ninguém a drena). -/
theorem workerless_never_drains :
    ∀ (parked cap : U64) (g : FlusherGate) (d : ParkedDebtPlan),
      (flusher_gate_plan false = ok g ∧
          parked_debt_plan parked cap = ok d) →
        g = FlusherGate.Workerless := by
  intro parked cap g d h
  have := (writer_workerless_gate_and_debt_iff false parked cap g d).mp h
  rcases this.1 with ⟨htrue, _⟩ | ⟨_, hg⟩
  · cases htrue
  · exact hg

/-- AS-IS dente: o gate ignora `attached` e sempre WorkerDrains —
    writer workerless dorme para sempre (RFC-0219 P2.2 plant). -/
theorem flusher_gate_plan_as_is_always_drains :
    ∀ (attached : Bool),
      flusher_gate_plan_as_is attached = ok FlusherGate.WorkerDrains := by
  intro attached
  unfold flusher_gate_plan_as_is
  rfl
