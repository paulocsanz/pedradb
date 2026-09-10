-- Theorems over the Aeneas extract of production ae_kernel.rs (F16).
-- RFC-0053 P2.1: second machine (not the Verus twin).
import Aeneas
import AeKernel
open Aeneas Std Result
open pedra_aeneas_ae_kernel

/-- Same-term existing entry is Keep. -/
theorem ae_keep_if_same_term (idx term commit last : U64) :
    ae_entry_action idx term (some term) commit last = ok .Keep := by
  unfold ae_entry_action
  simp

/-- Same-term existing entry is Keep (production unit-test point). -/
theorem ae_keep_same_index_term :
    ae_entry_action (2#u64) (7#u64) (some (7#u64)) (1#u64) (5#u64)
      = ok .Keep :=
  ae_keep_if_same_term (2#u64) (7#u64) (1#u64) (5#u64)

/-- F16: conflict at commit index is Refuse, not truncate. -/
theorem ae_refuse_conflict_at_commit :
    ae_entry_action (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
      = ok .Refuse := by
  unfold ae_entry_action
  simp

/-- Uncommitted term conflict truncates. -/
theorem ae_truncate_conflict_after_commit :
    ae_entry_action (3#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
      = ok .TruncateAndInstall := by
  unfold ae_entry_action
  simp

/-- F16 teeth: AS-IS mutant rewrites a committed slot. -/
theorem as_is_rewrites_committed :
    ae_entry_action (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
        = ok .Refuse ∧
      ae_entry_action_as_is_rewrite_committed
          (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
        = ok .TruncateAndInstall := by
  constructor
  · exact ae_refuse_conflict_at_commit
  · unfold ae_entry_action_as_is_rewrite_committed
    simp

/-- Catalog entry: an AppendEntries ack succeeds exactly when the log
    is clean, or it is dirty and the persist succeeded (F48 — a dirty
    log whose persist failed never acks ok). -/
theorem ae_ack_success_ok_iff_clean_or_dirty_persisted :
    ∀ (log_dirty : Bool) (persist_ok : Bool),
      (ae_ack_success log_dirty persist_ok = ok true)
        ↔ (log_dirty = false ∨ persist_ok = true) := by
  intro log_dirty persist_ok
  unfold ae_ack_success
  constructor
  · intro h
    split at h
    · next c1 =>
      simp at h
      exact Or.inr h
    · next c1 => exact Or.inl (by simp at c1; exact c1)
  · rintro (h1 | hc)
    · rw [if_neg (by simp [h1])]
    · split
      · next _ => rw [hc]
      · rfl
