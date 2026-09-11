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
