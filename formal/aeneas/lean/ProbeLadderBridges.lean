-- RFC-0199 (P0.3) → RFC-0204 (P1.1): the semantic bridges of the
-- point-get probe ladder. The Nat count twins and the REGISTERED
-- bound theorems (`probe_order_covering_work_bound`,
-- `point_get_probes_le_levels_l0_max`) moved to the MACHINE-EMITTED
-- `ProbeOrderCoveringDerived.lean` / `ScalePredictDerived.lean`
-- (single emitter: scripts/ratchet/derive_count_annotations.py;
-- drift-gated by lean_extracts.sh --check). What stays HERE, human
-- by design, are the bridges that tie the twins to the real Aeneas
-- extracts: every `cont` step of the probe ladder consumes exactly
-- one candidate (index += 1, and the add must not overflow) and only
-- happens while candidates remain; the ladder only reports `done`
-- once the candidate index has passed `newest_first.len`; every
-- `cont` step of the covering scan advances exactly one position
-- (and only while positions remain), so each candidate pays at most
-- one `by_lo` scan — the ladder's work is candidates × scan length,
-- never a walk over the store. The scale side pins the probe count
-- itself: `point_get_probes` is the exact saturating sum
-- `levels + l0_covering`, so the saturating-add bridge below (now
-- public) carries the emitted `ScalePredictDerived.lean` bound.
import Aeneas
import ProbeOrderKernel
import ScaleKernel
open Aeneas Aeneas.Std Result ControlFlow
open pedra_aeneas_probe_order_kernel pedra_aeneas_scale_kernel

/-! ## Bridges to the real extract (human, declared) -/

/-- ok chains: a bind equal to an ok value forces the bound operation to
have returned ok (Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => simp at h
  | div => simp at h

/-- An ok `cont` of a pair payload injects on both components. -/
private theorem ok_cont_pair_inj {α β γ : Type} {x1 : α} {y1 : β} {x2 : α} {y2 : β}
    (h : (ok (ControlFlow.cont ((x1, y1) : α × β)) : Result (ControlFlow (α × β) γ))
      = ok (ControlFlow.cont (x2, y2))) :
    x1 = x2 ∧ y1 = y2 := by
  injection h with hc
  injection hc with hp
  injection hp with hx hy
  exact ⟨hx, hy⟩

/-- An ok `cont` of a usize payload injects. -/
private theorem ok_cont_usize_inj {x1 x2 : Std.Usize}
    (h : (ok (ControlFlow.cont x1) : Result (ControlFlow Std.Usize Std.Usize))
      = ok (ControlFlow.cont x2)) : x1 = x2 := by
  injection h with h1
  injection h1

/-- ok-outcome clashes: a `done` step is never a `cont` step. -/
private theorem ok_done_ne_cont {α β : Type} {x : α} {y : β} :
    ¬ (ok (ControlFlow.done x) = ok (ControlFlow.cont y)) := by
  intro he
  injection he with h1
  injection h1

/-- Bridge: every `cont` step of the real covering scan advances exactly
one position — and only while positions remain in `by_lo` (when the
increment itself would overflow, the step is not a `cont` at all). -/
theorem covering_pos_body_cont_step :
    ∀ (by_lo : Slice Std.Usize) (i pos pos2 : Std.Usize),
      covering_pos_loop.body by_lo i pos = ok (ControlFlow.cont pos2) →
      ∃ pos1, pos + 1#usize = ok pos1 ∧ pos2 = pos1 ∧
        pos.val < (Slice.len by_lo).val := by
  intro by_lo i pos pos2 h
  unfold covering_pos_loop.body at h
  dsimp +zeta only at h
  split at h
  · next hlt =>
      obtain ⟨i2, _, h⟩ := bind_ok_inv _ _ _ h
      split at h
      · exact (ok_done_ne_cont h).elim
      · obtain ⟨pos1, hadd, h⟩ := bind_ok_inv _ _ _ h
        exact ⟨pos1, hadd, (ok_cont_usize_inj h).symm, (UScalar.lt_equiv _ _).mp hlt⟩
  · exact (ok_done_ne_cont h).elim

/-- Bridge: the covering scan only reports `done` at the position it
stopped — either the position ran past `by_lo`'s end, or the position
holds the sought candidate index (a hit stops the scan early, never
late). -/
theorem covering_pos_body_done :
    ∀ (by_lo : Slice Std.Usize) (i pos pos2 : Std.Usize),
      covering_pos_loop.body by_lo i pos = ok (ControlFlow.done pos2) →
      pos2 = pos ∧ (¬ (pos.val < (Slice.len by_lo).val) ∨
        ∃ w, Slice.index_usize by_lo pos = ok w ∧ w = i) := by
  intro by_lo i pos pos2 h
  unfold covering_pos_loop.body at h
  dsimp +zeta only at h
  split at h
  · obtain ⟨i2, hind, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · injection h with h1
      injection h1 with hpos
      exact ⟨hpos.symm, Or.inr ⟨i2, hind, ‹i2 = i›⟩⟩
    · obtain ⟨pos1, _, h⟩ := bind_ok_inv _ _ _ h
      exact (ok_done_ne_cont h.symm).elim
  · rename_i hge
    injection h with h1
    injection h1 with hpos
    refine ⟨hpos.symm, Or.inl ?_⟩
    exact fun hlt => hge (UScalar.lt_imp _ _ hlt)

/-- Bridge: every `cont` step of the real probe ladder consumes exactly
one candidate — the resume index is precisely the incremented candidate
index (when the increment itself would overflow, the step is not a
`cont` at all) — and only while candidates remain. -/
theorem probe_ladder_body_cont_step :
    ∀ (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
      (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
      (out : alloc.vec.Vec Std.Usize) (out2 : alloc.vec.Vec Std.Usize)
      (k k2 : Std.Usize),
      probe_order_covering_loop.body newest_first by_lo prefix_end his key out k
        = ok (ControlFlow.cont (out2, k2)) →
      ∃ k1, k + 1#usize = ok k1 ∧ k2 = k1 ∧
        k.val < (Slice.len newest_first).val := by
  intro newest_first by_lo prefix_end his key out out2 k k2 h
  unfold probe_order_covering_loop.body at h
  dsimp +zeta only at h
  split at h
  · next hlt =>
      obtain ⟨i, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨pos, _, h⟩ := bind_ok_inv _ _ _ h
      split at h
      · obtain ⟨out1, _, h⟩ := bind_ok_inv _ _ _ h
        obtain ⟨k1, hadd, h⟩ := bind_ok_inv _ _ _ h
        obtain ⟨_, hidx⟩ := ok_cont_pair_inj h
        exact ⟨k1, hadd, hidx.symm, (UScalar.lt_equiv _ _).mp hlt⟩
      · split at h
        · obtain ⟨b, _, h⟩ := bind_ok_inv _ _ _ h
          split at h
          · obtain ⟨out1, _, h⟩ := bind_ok_inv _ _ _ h
            obtain ⟨k1, hadd, h⟩ := bind_ok_inv _ _ _ h
            obtain ⟨_, hidx⟩ := ok_cont_pair_inj h
            exact ⟨k1, hadd, hidx.symm, (UScalar.lt_equiv _ _).mp hlt⟩
          · obtain ⟨k1, hadd, h⟩ := bind_ok_inv _ _ _ h
            obtain ⟨_, hidx⟩ := ok_cont_pair_inj h
            exact ⟨k1, hadd, hidx.symm, (UScalar.lt_equiv _ _).mp hlt⟩
        · obtain ⟨k1, hadd, h⟩ := bind_ok_inv _ _ _ h
          obtain ⟨_, hidx⟩ := ok_cont_pair_inj h
          exact ⟨k1, hadd, hidx.symm, (UScalar.lt_equiv _ _).mp hlt⟩
  · exact (ok_done_ne_cont h).elim

/-- Bridge: the probe ladder only reports `done` once the candidate
index has passed `newest_first.len` — while candidates remain, every ok
outcome is a `cont`. -/
theorem probe_ladder_body_done_at_len :
    ∀ (newest_first : Slice Std.Usize) (by_lo : Slice Std.Usize)
      (prefix_end : Std.Usize) (his : Slice (Slice Std.U8)) (key : Slice Std.U8)
      (out out2 : alloc.vec.Vec Std.Usize) (k : Std.Usize),
      probe_order_covering_loop.body newest_first by_lo prefix_end his key out k
        = ok (ControlFlow.done out2) →
      ¬ (k.val < (Slice.len newest_first).val) := by
  intro newest_first by_lo prefix_end his key out out2 k h
  unfold probe_order_covering_loop.body at h
  dsimp +zeta only at h
  split at h
  · obtain ⟨i, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨pos, _, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · obtain ⟨out1, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨k1, _, h⟩ := bind_ok_inv _ _ _ h
      exact (ok_done_ne_cont h.symm).elim
    · split at h
      · obtain ⟨b, _, h⟩ := bind_ok_inv _ _ _ h
        split at h
        · obtain ⟨out1, _, h⟩ := bind_ok_inv _ _ _ h
          obtain ⟨k1, _, h⟩ := bind_ok_inv _ _ _ h
          exact (ok_done_ne_cont h.symm).elim
        · obtain ⟨k1, _, h⟩ := bind_ok_inv _ _ _ h
          exact (ok_done_ne_cont h.symm).elim
      · obtain ⟨k1, _, h⟩ := bind_ok_inv _ _ _ h
        exact (ok_done_ne_cont h.symm).elim
  · rename_i hge
    exact fun hlt => hge (UScalar.lt_imp _ _ hlt)

/-! ## Scale side: the saturating-add bridge (public — the emitted
`ScalePredictDerived.lean` composes it) -/

/-- The extracted saturating add never exceeds the plain sum. -/
theorem saturating_add_val_le (x y : Std.U64) :
    (core.num.U64.saturating_add x y).val ≤ x.val + y.val := by
  unfold core.num.U64.saturating_add UScalar.saturating_add
  simp only [UScalar.val, UScalarTy.numBits, UScalar.max, BitVec.toNat_ofNat]
  omega
