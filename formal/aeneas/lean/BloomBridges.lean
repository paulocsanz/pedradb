-- RFC-0199 (P2.3) → RFC-0204 (P1.2): the semantic bridges of the
-- bloom probe walk. The Nat count twin and the REGISTERED bound
-- theorem (`bloom_may_contain_work_bound`) moved to the
-- MACHINE-EMITTED `BloomMayContainDerived.lean` (single emitter:
-- scripts/ratchet/derive_count_annotations.py; drift-gated by
-- lean_extracts.sh --check); the loop is now ENROLLED in the same
-- parse that derives step_work (RFC-0203's deferral closed). What
-- stays HERE, human by design, are the bridges that tie the twin to
-- the real Aeneas extract: every `cont` step of the probe loop
-- advances the probe index by exactly one (the add must not
-- overflow), and the loop reports a completed pass (`done true`) only
-- once the index has reached the policy count k — a clear bit
-- short-circuits with `done false`, strictly below k.
import Aeneas
import BloomKernel
open Aeneas Aeneas.Std Result ControlFlow
open pedra_aeneas_bloom_kernel

/-! ## Bridges to the real extract (human, declared) -/

/-- ok chains: a bind equal to an ok value forces the bound operation
to have returned ok (Cf.lean's `bind_ok_inv`, restated for this
module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => simp at h
  | div => simp at h

/-- ok `cont` of a plain payload injects. -/
private theorem ok_cont_inj {α β : Type} {x1 x2 : α}
    (h : (ok (ControlFlow.cont x1 : ControlFlow α β)) = ok (ControlFlow.cont x2)) :
    x1 = x2 := by
  injection h with h1
  injection h1

/-- ok-outcome clashes: a `done` step is never a `cont` step. -/
private theorem ok_done_ne_cont {α β : Type} {x : β} {y : α} :
    ¬ ((ok (ControlFlow.done x : ControlFlow α β)) = ok (ControlFlow.cont y)) := by
  intro he
  injection he with h1
  injection h1

/-- A short-circuited pass (`done false`) is never a completed pass. -/
private theorem ok_done_false_ne_true {α : Type} :
    ¬ ((ok (ControlFlow.done false : ControlFlow α Bool))
        = ok (ControlFlow.done true)) := by
  intro he
  injection he with h1
  injection h1
  rename_i hb
  exact Bool.noConfusion hb

/-- Bridge: every `cont` step of the real probe loop advances the probe
index by exactly one — the resume index is precisely the incremented
probe index (when the increment itself would overflow, the step is not
a `cont` at all). -/
theorem bloom_may_contain_body_cont_step :
    ∀ (v : alloc.vec.Vec Std.U8) (i : Std.U32) (h1 h2 nbits : Std.U64)
      (i1 : Std.U32) (i2 : Std.U32),
      BloomFilter.may_contain_loop.body v i h1 h2 nbits i1
        = ok (ControlFlow.cont i2) →
      ∃ i4, i1 + 1#u32 = ok i4 ∧ i2 = i4 := by
  intro v i h1 h2 nbits i1 i2 h
  unfold BloomFilter.may_contain_loop.body at h
  split at h
  · obtain ⟨i2', _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨i3, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨b, _, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · obtain ⟨i4, hi4, h⟩ := bind_ok_inv _ _ _ h
      exact ⟨i4, hi4, (ok_cont_inj h).symm⟩
    · exact (ok_done_ne_cont h).elim
  · exact (ok_done_ne_cont h).elim

/-- Bridge: the probe loop reports a completed pass (`done true`) only
once the probe index has reached the policy count k — while probes
remain, every ok outcome is a `cont` or the short-circuit `done false`. -/
theorem bloom_may_contain_body_done_true_at_k :
    ∀ (v : alloc.vec.Vec Std.U8) (i : Std.U32) (h1 h2 nbits : Std.U64)
      (i1 : Std.U32),
      BloomFilter.may_contain_loop.body v i h1 h2 nbits i1
        = ok (ControlFlow.done true) →
      ¬ (i1.val < i.val) := by
  intro v i h1 h2 nbits i1 h
  unfold BloomFilter.may_contain_loop.body at h
  split at h
  · obtain ⟨i2', _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨i3, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨b, _, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · obtain ⟨i4, _, h⟩ := bind_ok_inv _ _ _ h
      exact (ok_done_ne_cont h.symm).elim
    · exact (ok_done_false_ne_true h).elim
  · rename_i hge
    exact fun hlt => hge (UScalar.lt_imp _ _ hlt)
