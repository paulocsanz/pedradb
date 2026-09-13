-- Cross-lib composition: the durability spine (RFC-0214 P2.1) —
-- env → wal → ack COMPOSED over the three registered write_ack
-- atoms (`catalog:write_ack_append`, `catalog:write_ack_barrier`,
-- `catalog:write_ack_ack`). Each spine step IS the Ok future of
-- its atom (the three `*_ok_step` bridges derive them from the iff
-- atoms — the extracted bodies are never opened for the ok
-- futures); the composition proves the ∀ sentence over ANY step
-- sequence: Inv-WAL after every step, and D1 (the acked prefix
-- survives every torn cut) at every reachable state. The D1 crown
-- opens `inv_wal` + `d1_modelo` (+ `crash_legal`, total) once —
-- the assembly work of the composition itself, not a leg re-proof.
-- Registration rule: a row needs a single catalog pair/entry; this
-- composition spans three atoms — same reason the other compose
-- libs carry no row (reason dated in findings).
import Aeneas
import WriteAck
open Aeneas.Std Result
open pedra_aeneas_write_ack_kernel

/-- One spine step: the Ok future of one promoted atom. -/
inductive spine_step :
    write_ack_kernel.WriteAckLedger → write_ack_kernel.WriteAckLedger → Prop
  | append (l : write_ack_kernel.WriteAckLedger) (n w : U64)
      (hadd : l.state.written + n = ok w) :
      spine_step l { l with state := { l.state with written := w } }
  | barrier (l : write_ack_kernel.WriteAckLedger) :
      spine_step l
        { l with state := { l.state with synced := l.state.written, written := l.state.written } }
  | ack (l : write_ack_kernel.WriteAckLedger)
      (hle : l.state.acked ≤ l.state.synced) :
      spine_step l { l with state := { l.state with acked := l.state.synced } }

/-- Any spine path from the cold ledger (k steps). -/
inductive spine_reach : Nat → write_ack_kernel.WriteAckLedger → Prop
  | init :
      spine_reach 0
        { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
  | step (k : Nat) (l l' : write_ack_kernel.WriteAckLedger)
      (h : spine_step l l') (hr : spine_reach k l) :
      spine_reach (k + 1) l'

/-- The append step is the atom's Ok future
    (atom `catalog:write_ack_append`). -/
theorem on_append_ok_step (l : write_ack_kernel.WriteAckLedger) (n w : U64)
    (hadd : l.state.written + n = ok w) :
    write_ack_kernel.WriteAckLedger.on_append l n
      = ok { l with state := { l.state with written := w } } :=
  (on_append_fate_iff l n w).mpr hadd

/-- The barrier step is the atom's Ok future — the env seam
    (`CrashModel.of` + honest `sync` via `fsync_promotes_pending`)
    composed inside the atom (atom `catalog:write_ack_barrier`). -/
theorem on_barrier_ok_step (l : write_ack_kernel.WriteAckLedger) :
    write_ack_kernel.WriteAckLedger.on_barrier l
      = ok { l with state := { l.state with synced := l.state.written, written := l.state.written } } :=
  (on_barrier_fate_iff l _).mpr rfl

/-- The ack step is the atom's Ok future, conditional on the
    invariant (atom `catalog:write_ack_ack`). -/
theorem on_ack_ok_step (l : write_ack_kernel.WriteAckLedger)
    (hle : l.state.acked ≤ l.state.synced) :
    write_ack_kernel.WriteAckLedger.on_ack l
      = ok { l with state := { l.state with acked := l.state.synced } } :=
  (on_ack_fate_iff l _).mpr ⟨hle, rfl⟩

/-- RFC-0214 P2.1: Inv-WAL is an invariant of EVERY spine path —
    the composition of the three registered atoms: after any
    sequence of append/barrier/ack steps from the cold ledger,
    `acked ⊆ synced ⊆ written`. -/
theorem spine_inv_every_reach :
    ∀ (k : Nat) (l : write_ack_kernel.WriteAckLedger),
      spine_reach k l →
        (l.state.acked ≤ l.state.synced ∧ l.state.synced ≤ l.state.written) := by
  intro k l hr
  induction hr with
  | init =>
      constructor <;> (show (0#u64 : U64) ≤ 0#u64; simp)
  | step k l l' hs _ ih =>
      obtain ⟨h1, h2⟩ := ih
      cases hs with
      | append n w hadd =>
          have hz := UScalar.add_equiv l.state.written n
          rw [hadd] at hz
          obtain ⟨_, hval, _⟩ := hz
          have h2v : l.state.synced.val ≤ l.state.written.val := by
            simpa [UScalar.le_equiv] using h2
          constructor
          · exact h1
          · show l.state.synced ≤ w
            rw [UScalar.le_equiv]
            omega
      | barrier =>
          constructor
          · dsimp only
            exact LE.le.trans h1 h2
          · dsimp only
            exact le_refl _
      | ack hle =>
          constructor
          · dsimp only
            exact le_refl _
          · dsimp only
            exact h2

/-- RFC-0214 P2.1 crown: at every reachable state, D1 holds for the
    acked prefix over EVERY torn cut — the env seam (`CrashModel.of`
    + `crash_legal`) composed through `d1_modelo`: no legal cut ever
    loses an acked byte. -/
theorem spine_d1_every_reach :
    ∀ (k : Nat) (l : write_ack_kernel.WriteAckLedger) (cut : U64),
      spine_reach k l → l.state.acked ≤ cut →
        d1_modelo_kernel.d1_modelo l.state l.state.acked cut = ok true := by
  intro k l cut hr hcut
  obtain ⟨h1, h2⟩ := spine_inv_every_reach k l hr
  have hIW : wal.wal_state_kernel.inv_wal l.state = ok true := by
    unfold wal.wal_state_kernel.inv_wal
    split
    · exact congrArg ok (decide_eq_true h2)
    · next hbad => exact absurd h1 hbad
  unfold d1_modelo_kernel.d1_modelo
  rw [hIW]
  simp only [bind_tc_ok]
  split
  · split
    · unfold env_crash_kernel.CrashModel.of
      simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
        core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt, liftFun2,
        bind_tc_ok]
      split
      · simp only [bind_tc_ok]
        unfold env_crash_kernel.crash_legal
        split
        · simp only [bind_tc_ok]
          split
          · exact congrArg ok (decide_eq_true hcut)
          · rfl
        · rfl
      · simp only [bind_tc_ok]
        unfold env_crash_kernel.crash_legal
        split
        · simp only [bind_tc_ok]
          split
          · exact congrArg ok (decide_eq_true hcut)
          · rfl
        · rfl
    · rfl
  · rfl
