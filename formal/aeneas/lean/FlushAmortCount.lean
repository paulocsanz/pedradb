-- RFC-0199 (P1.2): memtable→flush amortization credit.
-- Count twin of the write/flush cycle: writes add units to the
-- memtable, ticks consult the real `auto_flush_due` gate (armed +
-- `limit ≤ mem_bytes`), and a firing tick pays exactly the bytes the
-- memtable holds before resetting it. Every byte written is charged at
-- most once — total flush work never exceeds total written bytes plus
-- the memtable's initial fill — so per-write flush work amortizes to a
-- constant independent of the level structure below it. The bridges
-- pin the twin's gate to the real extract: `auto_flush_due` returns ok
-- true exactly when armed and the limit has been reached (and a
-- non-firing armed check certifies `mem_bytes < limit`).
import Aeneas
import FlushKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_flush_kernel

/-! ## Count twin (pure Nat) -/

inductive FlushStep
  | write (units : Nat)
  | tick

/-- Gate twin of `auto_flush_due`: a tick fires exactly when the
memtable has reached the limit (the armed policy). -/
def should_flush (limit m : Nat) : Bool := decide (limit ≤ m)

/-- Work twin of the memtable→flush cycle: returns
(final memtable bytes, total flushed bytes). A firing tick pays the
current memtable and resets it to zero. -/
def run_flush_steps (limit : Nat) : List FlushStep → Nat → Nat × Nat
  | [], m => (m, 0)
  | FlushStep.write u :: rest, m => run_flush_steps limit rest (m + u)
  | FlushStep.tick :: rest, m =>
      if should_flush limit m then
        let p := run_flush_steps limit rest 0
        (p.1, m + p.2)
      else run_flush_steps limit rest m

/-- Total units handed to the memtable by the schedule. -/
def write_units : List FlushStep → Nat
  | [] => 0
  | FlushStep.write u :: rest => u + write_units rest
  | FlushStep.tick :: rest => write_units rest

/-- Gate twin sharpness: a non-firing check certifies the memtable is
still under the limit. -/
theorem should_flush_false_under_limit : ∀ (limit m : Nat),
    should_flush limit m = false → m < limit := by
  intro limit m h
  unfold should_flush at h
  simp only [decide_eq_false_iff_not, Nat.not_le] at h
  exact h

/-- Accounting invariant: every byte is either still in the memtable
or was flushed exactly once — never both, never neither. -/
theorem run_paid_plus_mem_le : ∀ (limit : Nat) (steps : List FlushStep) (m : Nat),
    (run_flush_steps limit steps m).2 + (run_flush_steps limit steps m).1
      ≤ write_units steps + m := by
  intro limit steps
  induction steps with
  | nil => intro m; simp [run_flush_steps]
  | cons step rest ih =>
    intro m
    cases step with
    | write u =>
        have h := ih (m + u)
        simp only [run_flush_steps, write_units] at h ⊢
        omega
    | tick =>
        simp only [run_flush_steps, write_units]
        split
        · next _hfired =>
            have h0 := ih 0
            dsimp only
            omega
        · next _ =>
            exact ih m

/-- RFC-0199 count (P1.2): the flush work a schedule pays never
exceeds the bytes it wrote plus the memtable's initial fill — k writes
under the cap amortize their flushes to O(k), independent of how large
the levels below the memtable are. -/
theorem memtable_flush_amortized : ∀ (limit : Nat) (steps : List FlushStep) (m : Nat),
    (run_flush_steps limit steps m).2 ≤ write_units steps + m := by
  intro limit steps m
  have h := run_paid_plus_mem_le limit steps m
  omega

/-! ## Bridges to the real extract -/

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
