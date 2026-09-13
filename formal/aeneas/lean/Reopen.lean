-- Theorems over the Aeneas extract of production wal/reopen_kernel.rs
-- (RFC-0053 Y3.3 / RFC-0056 P1.2): second machine (not the Verus twin).
import Aeneas
import ReopenKernel
open Aeneas Std Result
open pedra_aeneas_reopen_kernel

/-- No damage ⇒ serve everything (no false refusal), for every profile. -/
theorem clean_reopen_serves_all (pt esc : Bool) :
    reopen_outcome ReopenDamage.None pt esc = ok ReopenOutcome.ServeAll := by
  cases pt <;> cases esc <;> rfl

/-- G8: FailClosed + damage ⇒ refuse — the visible map never silently
drops part of an acked suffix. -/
theorem fail_closed_refuses_every_damage (d : ReopenDamage) (esc : Bool)
    (h : d ≠ ReopenDamage.None) :
    reopen_outcome d false esc = ok ReopenOutcome.RefuseOpen := by
  cases d
  · exact absurd rfl h
  · cases esc <;> rfl
  · cases esc <;> rfl
  · cases esc <;> rfl
  · cases esc <;> rfl

/-- PointInTime + damage (not escalated) ⇒ report + serve prefix — the
discard is observable, never silent. -/
theorem pit_reports_unless_escalated (d : ReopenDamage) (h : d ≠ ReopenDamage.None) :
    reopen_outcome d true false = ok ReopenOutcome.ServePrefixReport ∧
      reopen_outcome d true true = ok ReopenOutcome.RefuseOpen := by
  cases d
  · exact absurd rfl h
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩

/-- AS-IS teeth: the swallow-damage mutant serves a damaged reopen
silently — exactly the G8 silent-wrong the fixed kernel refuses. -/
theorem as_is_swallows_damage (d : ReopenDamage) (h : d ≠ ReopenDamage.None) :
    reopen_outcome d false false = ok ReopenOutcome.RefuseOpen ∧
      reopen_outcome_as_is_silent d false false = ok ReopenOutcome.ServeAll := by
  cases d
  · exact absurd rfl h
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩
  · exact ⟨rfl, rfl⟩

/-- Catalog entry: a reopen serves all records exactly when the WAL
    carries no damage — any damage (truncated head, CRC mismatch, zero
    header, resync) is refused or served as a reported prefix, never
    silently served whole (F170/F171/G8). -/
theorem reopen_outcome_serve_all_iff_damage_none :
    ∀ (damage : ReopenDamage) (point_in_time : Bool) (escalated : Bool),
      (reopen_outcome damage point_in_time escalated
          = ok ReopenOutcome.ServeAll)
        ↔ (damage = ReopenDamage.None) := by
  intro damage point_in_time escalated
  unfold reopen_outcome
  cases damage with
  | None => exact ⟨fun _ => rfl, fun _ => rfl⟩
  | TruncatedHead =>
    constructor
    · intro h
      cases point_in_time <;> cases escalated <;> simp at h
    · rintro habsurd
      exact absurd habsurd (by simp)
  | Crc =>
    constructor
    · intro h
      cases point_in_time <;> cases escalated <;> simp at h
    · rintro habsurd
      exact absurd habsurd (by simp)
  | ZeroHeader =>
    constructor
    · intro h
      cases point_in_time <;> cases escalated <;> simp at h
    · rintro habsurd
      exact absurd habsurd (by simp)
  | Resync =>
    constructor
    · intro h
      cases point_in_time <;> cases escalated <;> simp at h
    · rintro habsurd
      exact absurd habsurd (by simp)

/-- RFC-0213 P2.1 2/2 (cap-only; the pair has carried a registered
    atom since RFC-0191 P2.3): the reopen fate for EVERY outcome value —
    no damage serves everything; damage with PointInTime not escalated
    serves the reported prefix; anything else refuses. The one-fate
    atom above (`reopen_outcome_serve_all_iff_damage_none`) pinned only
    ServeAll; this iff pins all three fates, so the AS-IS silent mutant
    (always ServeAll) is unreachable from the real kernel on every
    branch. -/
theorem reopen_outcome_fate_iff :
    ∀ (damage : ReopenDamage) (point_in_time escalated : Bool)
      (v : ReopenOutcome),
      (reopen_outcome damage point_in_time escalated = ok v) ↔
        ((damage = ReopenDamage.None ∧ v = ReopenOutcome.ServeAll) ∨
         (¬(damage = ReopenDamage.None) ∧ point_in_time = true ∧
            ¬(escalated = true) ∧
            v = ReopenOutcome.ServePrefixReport) ∨
         (¬(damage = ReopenDamage.None) ∧
            (¬(point_in_time = true) ∨ escalated = true) ∧
            v = ReopenOutcome.RefuseOpen)) := by
  intro damage point_in_time escalated v
  cases damage <;> cases point_in_time <;> cases escalated <;>
    simp [reopen_outcome, eq_comm]
/-- RFC-0218 P0.3 6/6 (atom `catalog:dictionary_link`): the destination
    of the reopen is EXACTLY the cited policy — without damage serves everything;
    damage with point-in-time unscaled serves the prefix reported;
    damage scaled or without point-in-time refuses to open. The AS-IS silencia
    (serves everything over damage — tooth planted in the crash-recover). -/
theorem reopen_outcome_flat_fate_iff :
    ∀ (damage : ReopenDamage) (point_in_time : Bool) (escalated : Bool)
      (outcome : ReopenOutcome),
      (reopen_outcome damage point_in_time escalated = ok outcome) ↔
        ((damage = ReopenDamage.None ∧
            outcome = ReopenOutcome.ServeAll) ∨
          (damage ≠ ReopenDamage.None ∧ point_in_time = true ∧
            escalated = true ∧ outcome = ReopenOutcome.RefuseOpen) ∨
          (damage ≠ ReopenDamage.None ∧ point_in_time = true ∧
            escalated = false ∧
            outcome = ReopenOutcome.ServePrefixReport) ∨
          (damage ≠ ReopenDamage.None ∧ point_in_time = false ∧
            outcome = ReopenOutcome.RefuseOpen)) := by
  intro damage point_in_time escalated outcome
  cases damage with
  | None =>
    constructor
    · intro hval
      simp only [reopen_outcome] at hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2 | h3 | h4)
      · subst hv
        rfl
      · exact absurd rfl h2.1
      · exact absurd rfl h3.1
      · exact absurd rfl h4.1
  | TruncatedHead | Crc | ZeroHeader | Resync =>
    constructor
    · intro hval
      simp only [reopen_outcome] at hval
      split at hval
      · next hpit =>
        split at hval
        · next hesc =>
          exact Or.inr (Or.inl ⟨fun h => ReopenDamage.noConfusion h,
            hpit, hesc, by injection hval with hv; exact hv.symm⟩)
        · next hesc =>
          simp only [Bool.not_eq_true] at hesc
          exact Or.inr (Or.inr (Or.inl ⟨fun h => ReopenDamage.noConfusion h,
            hpit, hesc, by injection hval with hv; exact hv.symm⟩))
      · next hpit =>
        simp only [Bool.not_eq_true] at hpit
        exact Or.inr (Or.inr (Or.inr
          ⟨fun h => ReopenDamage.noConfusion h, hpit,
            by injection hval with hv; exact hv.symm⟩))
    · rintro (h1 | ⟨hd, hpit, hesc, hv⟩ | ⟨hd, hpit, hesc, hv⟩ |
        ⟨hd, hpit, hv⟩)
      · exact absurd h1.1 (fun h => ReopenDamage.noConfusion h)
      · subst hpit
        subst hesc
        subst hv
        rfl
      · subst hpit
        subst hesc
        subst hv
        rfl
      · subst hpit
        subst hv
        rfl
