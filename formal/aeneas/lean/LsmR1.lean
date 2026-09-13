-- Theorems over Aeneas extract of lsm_r1_kernel.rs (RFC-0166 P2.1).
-- Charon --start-from catalog entries; compact nested-loop returns and
-- reopen_as_is hole patched to index loops in aeneas_lsm_r1.sh.
import Aeneas
import LsmR1Kernel
open Aeneas.Std Result
open pedra_aeneas_lsm_r1_kernel

/-- Catalog entry: reopen is identity. -/
theorem lsm_reopen_id (s) :
    lsm_reopen s = ok s := by
  unfold lsm_reopen
  rfl

/-- Catalog entry: compact of depth 0 is a no-op refuse. -/
theorem lsm_compact_depth_zero (s) :
    lsm_compact s (0#usize) = ok none := by
  unfold lsm_compact
  rfl

/-- AS-IS compact of depth 0 is also none (the mutant is the tomb drop). -/
theorem lsm_compact_as_is_depth_zero (s) :
    lsm_compact_as_is s (0#usize) = ok none := by
  unfold lsm_compact_as_is
  rfl

/-- Mint and write atoms are extracted defs (Array.repeat is not `decide`). -/
theorem lsm_state_of_is_def : True := by
  have _ := @lsm_state_of
  have _ := @lsm_write
  trivial

/-! ## RFC-0215 P0.2 — product crown in the atom rung (model ×4) -/

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- R1 model maintains: the inventory is consistente — probe and newest
concordam nthe same entry for the key (`inv_lsm`/`lsm_probe`/
`r1_newest` cited, bodies not reopened). -/
def r1m_ok_true (s : LsmState) (key : U64) : Prop :=
  ∃ b, inv_lsm s = ok b ∧
    (b = false ∨ b = true ∧
      ∃ o, lsm_probe s key = ok o ∧
        ∃ o1, r1_newest s key = ok o1 ∧
          core.option.Option.Insts.CoreCmpPartialEqOption.eq
            LsmEntry.Insts.CoreCmpPartialEqLsmEntry o o1 = ok true)

/-- R1 model viola: inventory broken — the probe and the newest
discordam over the key. -/
def r1m_mismatch (s : LsmState) (key : U64) : Prop :=
  ∃ b, inv_lsm s = ok b ∧ b = true ∧
    ∃ o, lsm_probe s key = ok o ∧
      ∃ o1, r1_newest s key = ok o1 ∧
        core.option.Option.Insts.CoreCmpPartialEqOption.eq
          LsmEntry.Insts.CoreCmpPartialEqLsmEntry o o1 = ok false

/-- RFC-0215 P0.2 2/4 (atom `catalog:r1_modelo`, entry `r1_modelo`):
the outcome of the R1 machine is exactly the decision the spec names —
`ok true` when probe and newest concordam (or the inventory nem
passes), `ok false` when discordam. The mutant AS-IS
(`r1_modelo_as_is`) probe with `lsm_probe_as_is` (the resurrection of the
delete de findings/2026-09-04-reopen-delete-resurrected); planta
three-teeth refuses. -/
theorem r1_modelo_fate_iff :
    ∀ (s : LsmState) (key : U64) (v : Bool),
      (r1_modelo s key = ok v) ↔
        ((v = true ∧ r1m_ok_true s key) ∨
          (v = false ∧ r1m_mismatch s key)) := by
  intro s key v
  constructor
  · intro hval
    unfold r1_modelo at hval
    obtain ⟨ b, hb, hval ⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hb' =>
        obtain ⟨ o, ho, hval ⟩ := bind_ok_inv _ _ _ hval
        obtain ⟨ o1, ho1, hval ⟩ := bind_ok_inv _ _ _ hval
        cases v with
        | true =>
            exact Or.inl ⟨rfl, ⟨b, hb, Or.inr ⟨hb', o, ho, o1, ho1, hval⟩⟩⟩
        | false =>
            exact Or.inr ⟨rfl, ⟨b, hb, hb', o, ho, o1, ho1, hval⟩⟩
    · next hb' =>
        have hbF : b = false := by simpa [Bool.not_eq_true] using hb'
        have hv : v = true := (Result.ok.inj hval).symm
        exact Or.inl ⟨hv, ⟨b, hb, Or.inl hbF⟩⟩
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, b, hb, hbr⟩ := hh
        subst hv
        unfold r1_modelo
        rw [hb]
        simp only [Aeneas.Std.bind_tc_ok]
        rcases hbr with hbF | ⟨hbt, o, ho, o1, ho1, heq⟩
        · rw [hbF, if_neg (by simp)]
        · rw [hbt, if_pos rfl]
          rw [ho]
          simp only [Aeneas.Std.bind_tc_ok]
          rw [ho1]
          simp only [Aeneas.Std.bind_tc_ok]
          exact heq
    | inr hh =>
        obtain ⟨hv, b, hb, hbt, o, ho, o1, ho1, heq⟩ := hh
        subst hv
        unfold r1_modelo
        rw [hb]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [hbt, if_pos rfl]
        rw [ho]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [ho1]
        simp only [Aeneas.Std.bind_tc_ok]
        exact heq

/-- RFC-0218 P1.1 8/10 (atom `catalog:lsm_compact`, entry
    `lsm_compact`): dispatch of compaction R1 is EXACTLY the
    cited dispatch — level 0 does not compact (ok none), level
    inside MAX_LEVELS enters the cited loop with drop_all_tombs
    false, level beyond MAX_LEVELS does not compact. The AS-IS passes
    drop_all_tombs true (derruba tombstones live — tooth planted). -/
theorem lsm_compact_fate_iff :
    ∀ (s : LsmState) (depth : Usize) (r : Option LsmState),
      (lsm_compact s depth = ok r) ↔
      ((depth = 0#usize ∧ r = none) ∨
       (¬(depth = 0#usize) ∧ depth < MAX_LEVELS ∧
         lsm_compact_src_loop false depth s depth = ok r) ∨
       (¬(depth = 0#usize) ∧ ¬(depth < MAX_LEVELS) ∧ r = none)) := by
  intro s depth r
  constructor
  · intro hval
    unfold lsm_compact at hval
    split at hval
    · next hz =>
      injection hval with hv
      exact Or.inl ⟨hz, hv.symm⟩
    · next hz =>
      split at hval
      · next hm => exact Or.inr (Or.inl ⟨hz, hm, hval⟩)
      · next hm =>
        injection hval with hv
        exact Or.inr (Or.inr ⟨hz, hm, hv.symm⟩)
  · rintro (⟨hz, hv⟩ | ⟨hz, hm, hs⟩ | ⟨hz, hm, hv⟩)
    · unfold lsm_compact
      rw [if_pos hz, hv]
    · unfold lsm_compact
      rw [if_neg hz, if_pos hm]
      exact hs
    · unfold lsm_compact
      rw [if_neg hz, if_neg hm, hv]

/-- RFC-0218 P1.1 9/10 (atom `catalog:lsm_probe`, entry
    `lsm_probe`): prove R1 is EXACTLY one step of the cited loop —
    the body in the level 0 or ends (done the) or goes down the level
    (cont i', resto cited). The AS-IS ignora the level (tooth
    planted). -/
theorem lsm_probe_fate_iff :
    ∀ (s : LsmState) (key : U64) (o : Option LsmEntry),
      (lsm_probe s key = ok o) ↔
      (∃ b, lsm_probe_loop.body s key 0#usize = ok b ∧
        ((b = ControlFlow.done o) ∨
         (∃ i', b = ControlFlow.cont i' ∧ lsm_probe_loop s key i' = ok o))) := by
  intro s key o
  constructor
  · intro hval
    unfold lsm_probe at hval
    unfold lsm_probe_loop at hval
    rw [Aeneas.Std.loop.eq_def] at hval
    cases hb : lsm_probe_loop.body s key 0#usize with
    | ok b =>
      rw [hb] at hval
      cases b with
      | cont i' =>
        exact ⟨ControlFlow.cont i', rfl, Or.inr ⟨i', rfl, hval⟩⟩
      | done o' =>
        injection hval with hv
        exact ⟨ControlFlow.done o', rfl, Or.inl (by rw [hv])⟩
    | fail e => simp [hb] at hval
    | div => simp [hb] at hval
  · rintro ⟨b, hb, (hdone | ⟨i', hcont, hloop⟩)⟩
    · unfold lsm_probe
      unfold lsm_probe_loop
      rw [Aeneas.Std.loop.eq_def]
      rw [hb, hdone]
    · unfold lsm_probe
      unfold lsm_probe_loop
      rw [Aeneas.Std.loop.eq_def]
      rw [hb, hcont]
      exact hloop

/-- RFC-0218 P1.1 10/10 (atom `catalog:lsm_reopen`, entry
    `lsm_reopen`): reabrir R1 is EXACTLY the identidade cited — the
    state exits intacto (`r = s`). The AS-IS reabre by the loop that
    empties levels (tooth planted). -/
theorem lsm_reopen_fate_iff :
    ∀ (s : LsmState) (r : LsmState),
      (lsm_reopen s = ok r) ↔ (r = s) := by
  intro s r
  constructor
  · intro hval
    unfold lsm_reopen at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
