-- Theorems over the Aeneas extract of production apply_kernel.rs
-- (RFC-0053 Y2.2 / RFC-0056 P1.2): second machine (not the Verus twin).
import Aeneas
import ApplyKernel
open Aeneas Std Result
open pedra_aeneas_apply_kernel

/-- Closed form: the apply loop only ever advances inside the contiguous
committed prefix (F10-apply). -/
theorem apply_advance_closed_form (la ci : Std.U64) (present : Bool) :
    apply_advance la ci present =
      (if la >= ci then ok ApplyAction.Done
       else if present then ok ApplyAction.Apply else ok ApplyAction.Stop) := by
  unfold apply_advance
  split
  · rfl
  · split <;> rfl

/-- At/below commit ⇒ Done (no runaway apply past commit_index). -/
theorem done_at_and_above_commit :
    apply_advance (5#u64) (5#u64) true = ok ApplyAction.Done ∧
      apply_advance (9#u64) (5#u64) false = ok ApplyAction.Done := by
  constructor <;> rfl

/-- A hole in the log ⇒ Stop — the store never applies outside the
contiguous committed prefix. -/
theorem missing_entry_stops :
    apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  rfl

/-- Present entry below commit ⇒ Apply. -/
theorem present_entry_applies :
    apply_advance (1#u64) (2#u64) true = ok ApplyAction.Apply := by
  rfl

/-- AS-IS teeth: the skip-holes mutant applies a missing entry — exactly
the hole the fixed kernel stops at. -/
theorem as_is_applies_hole :
    apply_advance_as_is_skip_holes (1#u64) (2#u64) false = ok ApplyAction.Apply ∧
      apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  constructor <;> rfl
