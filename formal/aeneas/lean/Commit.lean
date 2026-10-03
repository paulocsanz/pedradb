-- Theorems over the Aeneas extract of production commit_kernel.rs (F10 / F23).
-- RFC-0053 P2.1: second machine (not the Verus twin).
import Aeneas
import CommitKernel
open Aeneas Std Result
open pedra_aeneas_commit_kernel

private theorem recover_commit_eq_min (loaded log_last : U64) :
    recover_commit loaded log_last =
      core.cmp.Ord.min.trait_default core.cmp.OrdU64 loaded log_last := rfl

/-- Closed form: majority ∧ same term (F23). -/
theorem may_commit_at_iff (it ct : U64) (maj : Bool) :
    may_commit_at it ct maj = ok (maj && decide (it = ct)) := by
  unfold may_commit_at
  cases maj <;> simp

/-- F10: recover never exceeds the on-disk log last index (concrete + closed min). -/
theorem recover_commit_caps_examples :
    recover_commit (5#u64) (3#u64) = ok (3#u64) ∧
      recover_commit (2#u64) (9#u64) = ok (2#u64) ∧
      recover_commit (0#u64) (0#u64) = ok (0#u64) := by
  unfold recover_commit
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS F23 teeth: majority alone would commit a previous-term index. -/
theorem as_is_commits_prev_term :
    may_commit_at (2#u64) (3#u64) true = ok false ∧
      may_commit_at_as_is (2#u64) (3#u64) true = ok true := by
  constructor
  · unfold may_commit_at; simp
  · rfl

/-- AS-IS F10 teeth: recover can promote an uncommitted suffix. -/
theorem as_is_recover_promotes_suffix :
    recover_commit_as_is (1#u64) (5#u64) = ok (5#u64) ∧
      recover_commit (1#u64) (5#u64) = ok (1#u64) := by
  constructor
  · rfl
  · unfold recover_commit
    simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
      core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- RFC-0208 P0.2 (seam raft 2/2, atom `catalog:commit_raft`): a
    propose gets its ack EXACTLY when the entry's index is at or below
    the commit index (already committed) — no phantom ack for an
    uncommitted index, and a committed one is never left unacked.
    The AS-IS `ok true` mutant acks everything. -/
theorem propose_ack_ok_fate_iff :
    ∀ (index : U64) (commit_index : U64) (v : Bool),
      (propose_ack_ok index commit_index = ok v) ↔
        ((v = true ∧ commit_index >= index)
          ∨ (v = false ∧ ¬(commit_index >= index))) := by
  intro index commit_index v
  unfold propose_ack_ok
  cases hd : decide (commit_index >= index) with
  | true =>
    have hP := of_decide_eq_true hd
    cases v <;> simp [hP]
  | false =>
    have hnP := of_decide_eq_false hd
    cases v <;> simp [hnP]

/-- RFC-0218 P2.2 (átomo `catalog:raft_recover_applied`, entrada
    `recover_last_applied`): o applied recuperado no open é exatamente
    a constante citada zero — o índice aplicado NUNCA é confiado do
    disco; o replay recomeça do zero. O AS-IS reanima o último applied
    gravado (dente plantado). -/
theorem recover_last_applied_fate_iff :
    ∀ (v : U64), (recover_last_applied = ok v) ↔ (v = 0#u64) := by
  intro v
  constructor
  · intro hval
    unfold recover_last_applied at hval
    injection hval with hv
    exact hv.symm
  · rintro rfl
    unfold recover_last_applied
    rfl
