-- RFC-0199 (P0.2): counting-ladder credit over the lsm_r1 compact walk.
-- Count twins are hand-written Nat mirrors of the loop iteration counts
-- (declared debt until the P2.1 cost tool derives them mechanically).
-- The bridges tie the twins to the real Aeneas extract: every `cont` step
-- of the inner loop consumes exactly one entry (index += 1, and the add
-- must not overflow), the loop only reports `done (some _)` once the
-- index has passed the source level's logical length, and the outer
-- drain loop steps down exactly one level per `cont` (the subtract must
-- not underflow) — the walk is one-pass over the stored entries.
import Aeneas
import LsmR1Kernel
open Aeneas Aeneas.Std Result ControlFlow
open pedra_aeneas_lsm_r1_kernel

/-! ## Count twins (pure Nat) -/

/-- Work twin of the compact inner loop: iterations while `remaining`
entries are left to consume (`remaining` is the loop's own decreasing
measure `src_len - i`) — one step per consumed entry. -/
def lsm_compact_steps : Nat → Nat
  | 0 => 0
  | remaining + 1 => 1 + lsm_compact_steps remaining

/-- Logical length (`len`) of level `lvl` of `s`; 0 when out of range. -/
def lsm_level_len (s : LsmState) (lvl : Nat) : Nat :=
  match s.levels.val[lvl]? with
  | some l => l.len.val
  | none => 0

/-- Entries stored strictly below `depth` — the compact drain set. -/
def lsm_entries_below (s : LsmState) : Nat → Nat
  | 0 => 0
  | lvl + 1 => lsm_level_len s lvl + lsm_entries_below s lvl

/-- Work twin of the whole compact walk: one inner pass over each source
level's stored entries plus one bookkeeping iteration per drained level. -/
def lsm_compact_src_steps (s : LsmState) : Nat → Nat
  | 0 => 0
  | lvl + 1 => lsm_compact_steps (lsm_level_len s lvl) + 1 + lsm_compact_src_steps s lvl

/-- Inner twin bound: one iteration per remaining entry, no more. -/
theorem lsm_compact_steps_le : ∀ (remaining : Nat),
    lsm_compact_steps remaining ≤ remaining := by
  intro remaining
  induction remaining with
  | zero => simp [lsm_compact_steps]
  | succ d ih => simp only [lsm_compact_steps]; omega

/-- RFC-0199 count (P0.2): the compact walk's work twin never exceeds the
entries stored below the target level plus one bookkeeping iteration per
drained level — one-pass over the stored data, independent of how often
compact is called or how big the deeper (untouched) levels are. -/
theorem lsm_compact_work_bound : ∀ (s : LsmState) (depth : Nat),
    lsm_compact_src_steps s depth ≤ lsm_entries_below s depth + depth := by
  intro s depth
  induction depth with
  | zero => simp [lsm_compact_src_steps, lsm_entries_below]
  | succ lvl ih =>
    have h := lsm_compact_steps_le (lsm_level_len s lvl)
    simp only [lsm_compact_src_steps, lsm_entries_below]
    omega

/-! ## Bridges to the real extract -/

/-- ok chains: a bind equal to an ok value forces the bound operation to
have returned ok (Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => simp at h
  | div => simp at h

/-- An ok `cont` of a pair payload injects on both components. -/
private theorem ok_cont_inj {α β γ : Type} {x1 : α} {y1 : β} {x2 : α} {y2 : β}
    (h : (ok (ControlFlow.cont ((x1, y1) : α × β)) : Result (ControlFlow (α × β) γ))
      = ok (ControlFlow.cont (x2, y2))) :
    x1 = x2 ∧ y1 = y2 := by
  injection h with hc
  injection hc with hp
  injection hp with hx hy
  exact ⟨hx, hy⟩

/-- ok-outcome clashes: a `done` step is never a `cont` step. -/
private theorem ok_done_ne_cont {α β : Type} {x : Option α} {y : β} :
    ¬ (ok (ControlFlow.done x) = ok (ControlFlow.cont y)) := by
  intro he
  injection he with h1
  injection h1

/-- A capacity `done none` is never a completed `done (some _)`. -/
private theorem ok_done_none_ne_some {α β : Type} {x : α} :
    ¬ ((ok (ControlFlow.done (none : Option α)) :
          Result (ControlFlow β (Option α)))
      = ok (ControlFlow.done (some x))) := by
  intro he
  injection he with h1
  injection h1 with h2
  injection h2

/-- Bridge: every `cont` step of the real inner loop consumes exactly one
entry — the resume index is precisely the incremented loop index (when
the increment itself would overflow, the step is not a `cont` at all). -/
theorem lsm_compact_inner_body_cont_step :
    ∀ (drop_all_tombs : Bool) (depth : Std.Usize) (src : LsmLevel)
      (out : LsmState) (i : Std.Usize) (out2 : LsmState) (i2 : Std.Usize),
    lsm_compact_inner_loop.body drop_all_tombs depth src out i
      = ok (ControlFlow.cont (out2, i2)) →
      ∃ i1, i + 1#usize = ok i1 ∧ i2 = i1 := by
  intro drop_all_tombs depth src out i out2 i2 h
  unfold lsm_compact_inner_loop.body at h
  split at h
  · obtain ⟨e, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨dst, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨max1, _, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · -- tombstone retired (bottom level / drop-all): skip the entry
      obtain ⟨dst1, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨levels1, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨i1, hi1, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨_, hidx⟩ := ok_cont_inj h
      exact ⟨i1, hi1, hidx.symm⟩
    · -- put branch
      obtain ⟨p, _, h⟩ := bind_ok_inv _ _ _ h
      cases p with
      | mk ok1 dst1 =>
      cases ok1 with
      | false => exact (ok_done_ne_cont h).elim
      | true =>
        obtain ⟨levels1, _, h⟩ := bind_ok_inv _ _ _ h
        obtain ⟨i1, hi1, h⟩ := bind_ok_inv _ _ _ h
        obtain ⟨_, hidx⟩ := ok_cont_inj h
        exact ⟨i1, hi1, hidx.symm⟩
  · exact (ok_done_ne_cont h).elim

/-- Bridge: the inner loop only reports a completed pass (`done (some _)`)
once the index has passed the source level's logical length — while
entries remain, every ok outcome is a `cont` or a capacity `done none`. -/
theorem lsm_compact_inner_body_done_at_len :
    ∀ (drop_all_tombs : Bool) (depth : Std.Usize) (src : LsmLevel)
      (out : LsmState) (out2 : LsmState) (i : Std.Usize),
    lsm_compact_inner_loop.body drop_all_tombs depth src out i
      = ok (ControlFlow.done (some out2)) →
      ¬ (i.val < src.len.val) := by
  intro drop_all_tombs depth src out out2 i h
  unfold lsm_compact_inner_loop.body at h
  split at h
  · obtain ⟨e, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨dst, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨max1, _, h⟩ := bind_ok_inv _ _ _ h
    split at h
    · obtain ⟨dst1, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨levels1, _, h⟩ := bind_ok_inv _ _ _ h
      obtain ⟨i1, _, h⟩ := bind_ok_inv _ _ _ h
      exact (ok_done_ne_cont h.symm).elim
    · obtain ⟨p, _, h⟩ := bind_ok_inv _ _ _ h
      cases p with
      | mk ok1 dst1 =>
      cases ok1 with
      | false =>
        have h' : ok (ControlFlow.done (none : Option LsmState))
            = ok (ControlFlow.done (some out2)) := h
        exact (ok_done_none_ne_some h').elim
      | true =>
        obtain ⟨levels1, _, h⟩ := bind_ok_inv _ _ _ h
        obtain ⟨i1, _, h⟩ := bind_ok_inv _ _ _ h
        exact (ok_done_ne_cont h.symm).elim
  · rename_i hge
    exact fun hlt => hge (UScalar.lt_imp _ _ hlt)

/-- Bridge: every `cont` step of the real drain loop moves down exactly
one source level (when the decrement would underflow at 0, the step is
not a `cont` at all — the walk cannot run past the level stack). -/
theorem lsm_compact_src_body_cont_step :
    ∀ (drop_all_tombs : Bool) (depth : Std.Usize) (out : LsmState)
      (src_lvl : Std.Usize) (out2 : LsmState) (lvl2 : Std.Usize),
    lsm_compact_src_loop.body drop_all_tombs depth out src_lvl
      = ok (ControlFlow.cont (out2, lvl2)) →
      ∃ lvl1, src_lvl - 1#usize = ok lvl1 ∧ lvl2 = lvl1 := by
  intro drop_all_tombs depth out src_lvl out2 lvl2 h
  unfold lsm_compact_src_loop.body at h
  split at h
  · obtain ⟨src_lvl1, hsub, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨src, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨empty, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨levels1, _, h⟩ := bind_ok_inv _ _ _ h
    obtain ⟨o, _, h⟩ := bind_ok_inv _ _ _ h
    cases o with
    | none => exact (ok_done_ne_cont h).elim
    | some out2' =>
      obtain ⟨_, hidx⟩ := ok_cont_inj h
      exact ⟨src_lvl1, hsub, hidx.symm⟩
  · exact (ok_done_ne_cont h).elim
