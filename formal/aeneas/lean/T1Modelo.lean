-- Theorems over Aeneas extract of t1_modelo_kernel.rs (RFC-0166 P2.2).
import Aeneas
import T1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_t1_modelo_kernel

/-- Catalog entry: empty TX recovers to T1. -/
theorem t1_modelo_empty :
    t1_modelo_kernel.t1_modelo
      { staged := 0#u64, visible := 0#u64, committed := false,
        aborted := false, fenced := false } = ok true := by
  unfold t1_modelo_kernel.t1_modelo
  unfold t1_modelo_kernel.tx_recover
  unfold txn_kernel.leftover_fate
  unfold t1_modelo_kernel.t1_holds_of
  rfl

/-- AS-IS dente: mid-apply partial visibility is not recovered. -/
theorem t1_modelo_as_is_dente :
    t1_modelo_kernel.t1_modelo_as_is
      { staged := 2#u64, visible := 1#u64, committed := false,
        aborted := false, fenced := false } = ok false := by
  unfold t1_modelo_kernel.t1_modelo_as_is
  unfold t1_modelo_kernel.tx_recover_as_is
  unfold txn_kernel.leftover_txn_is_aborted_as_is
  unfold txn_kernel.leftover_fate_as_is
  unfold t1_modelo_kernel.t1_holds_of
  rfl

/-! ## RFC-0215 P0.2 — coroa de produto no degrau átomo (modelo ×4) -/

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- T1 modelo mantém: a recuperação devolve estado que passa o
preditor (`t1_holds_of` citado, corpo não reaberto). -/
def t1m_holds (s : t1_modelo_kernel.TxState) : Prop :=
  ∃ ts, t1_modelo_kernel.tx_recover s = ok ts ∧
    t1_modelo_kernel.t1_holds_of ts = ok true

/-- T1 modelo viola: a tx recuperada quebra o preditor
all-or-nothing (efeito parcial visível). -/
def t1m_violates (s : t1_modelo_kernel.TxState) : Prop :=
  ∃ ts, t1_modelo_kernel.tx_recover s = ok ts ∧
    t1_modelo_kernel.t1_holds_of ts = ok false

/-- RFC-0215 P0.2 3/4 (atom `catalog:t1_modelo`, entry `t1_modelo`):
o desfecho da máquina T1 é exatamente a decisão que o spec nomeia —
`ok true` quando a tx recuperada passa `t1_holds_of`, `ok false`
quando a quebra. O mutante AS-IS (`t1_modelo_as_is`) recupera com
`tx_recover_as_is`/`leftover_*_as_is`; planta três-dentes recusa. -/
theorem t1_modelo_fate_iff :
    ∀ (s : t1_modelo_kernel.TxState) (v : Bool),
      (t1_modelo_kernel.t1_modelo s = ok v) ↔
        ((v = true ∧ t1m_holds s) ∨ (v = false ∧ t1m_violates s)) := by
  intro s v
  constructor
  · intro hval
    unfold t1_modelo_kernel.t1_modelo at hval
    obtain ⟨ ts, hts, hval ⟩ := bind_ok_inv _ _ _ hval
    cases v with
    | true => exact Or.inl ⟨rfl, ⟨ts, hts, hval⟩⟩
    | false => exact Or.inr ⟨rfl, ⟨ts, hts, hval⟩⟩
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, ts, hts, hhold⟩ := hh
        subst hv
        unfold t1_modelo_kernel.t1_modelo
        rw [hts]
        simp only [Aeneas.Std.bind_tc_ok]
        exact hhold
    | inr hh =>
        obtain ⟨hv, ts, hts, hviol⟩ := hh
        subst hv
        unfold t1_modelo_kernel.t1_modelo
        rw [hts]
        simp only [Aeneas.Std.bind_tc_ok]
        exact hviol
