-- RFC-0199 (P1.2) → RFC-0204 (P1.1): the semantic bridges of the
-- memtable→flush amortization credit. The Nat count twins and the
-- REGISTERED bound theorem (`memtable_flush_amortized`) moved to the
-- MACHINE-EMITTED `AutoFlushDueDerived.lean` (single emitter:
-- scripts/ratchet/derive_count_annotations.py; drift-gated by
-- lean_extracts.sh --check). What stays HERE, human by design, are
-- the bridges that pin the twin's gate to the real extract:
-- `auto_flush_due` returns ok true exactly when armed and the limit
-- has been reached, and a non-firing armed check certifies
-- `mem_bytes < limit`.
import Aeneas
import FlushKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_flush_kernel

/-! ## Bridges to the real extract (human, declared) -/

/-- Bridge: the real `auto_flush_due` fires exactly when the policy is
armed and the memtable has reached the limit. -/
theorem auto_flush_due_fires_iff : ∀ (mem_bytes : Std.U64) (armed : Bool) (limit : Std.U64),
    auto_flush_due mem_bytes armed limit = ok true ↔
      armed = true ∧ limit.val ≤ mem_bytes.val := by
  intro mem_bytes armed limit
  unfold auto_flush_due
  cases armed with
  | true =>
      constructor
      · intro h
        injection h with hb
        exact ⟨rfl, (UScalar.le_equiv _ _).mp (of_decide_eq_true hb)⟩
      · intro ⟨_, hle⟩
        have hp : limit ≤ mem_bytes := (UScalar.le_equiv _ _).mpr hle
        have hb : (decide (limit ≤ mem_bytes) : Bool) = true := decide_eq_true hp
        exact congrArg ok hb
  | false =>
      simp

/-- Bridge: an armed `auto_flush_due` that does not fire certifies the
memtable is still under the limit — the twin's
`should_flush_false_under_limit` gate. -/
theorem auto_flush_due_hold_under_limit : ∀ (mem_bytes : Std.U64) (limit : Std.U64),
    auto_flush_due mem_bytes true limit = ok false → mem_bytes.val < limit.val := by
  intro mem_bytes limit h
  unfold auto_flush_due at h
  injection h with hb
  simp only [decide_eq_false_iff_not] at hb
  have : ¬ (limit.val ≤ mem_bytes.val) := by
    simpa using hb
  omega
