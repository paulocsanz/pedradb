-- Theorems over Aeneas extract of wal/wal_state_kernel.rs (RFC-0166 P1.2).
-- RFC-0191 P2.1: Inv-WAL one-step preservation + D1 corollary.
import Aeneas
import WalStateKernel
import WriteAdmission
open Aeneas.Std Result
open pedra_aeneas_wal_state_kernel
open pedra_aeneas_write_admission_kernel

/-- Catalog entry: acked ⊆ synced ⊆ written. -/
theorem inv_wal_well_formed :
    wal.wal_state_kernel.inv_wal
      { acked := 0#u64, synced := 4#u64, written := 10#u64 } = ok true := by
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- AS-IS tooth: acked past the barrier still admits. -/
theorem inv_wal_as_is_tooth :
    wal.wal_state_kernel.inv_wal_as_is
      { acked := 5#u64, synced := 0#u64, written := 10#u64 } = ok true := by
  unfold wal.wal_state_kernel.inv_wal_as_is
  rfl

/-- Catalog corollary: a legal cut is at/after acked.
    Unfolds `CrashModel.of` + `crash_legal`. -/
theorem acked_survives_legal_cut :
    wal.wal_state_kernel.acked_survives_every_legal_crash
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (7#u64) = ok true := by
  unfold wal.wal_state_kernel.acked_survives_every_legal_crash
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS tooth: a cut below the barrier is treated as legal and fails. -/
theorem acked_survives_as_is_tooth :
    wal.wal_state_kernel.acked_survives_as_is
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (3#u64) = ok false := by
  unfold wal.wal_state_kernel.acked_survives_as_is
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- Honest `wal_sync` unfolds `env_crash_kernel.sync` and promotes. -/
theorem wal_sync_honest_promotes :
    wal.wal_state_kernel.wal_sync
      { acked := 0#u64, synced := 0#u64, written := 96#u64 }
      env_crash_kernel.SyncHonesty.Honest
      = ok { acked := 0#u64, synced := 96#u64, written := 96#u64 } := by
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-- Lying `wal_sync`: same callee, watermark stays. -/
theorem wal_sync_lying_does_not_promote :
    wal.wal_state_kernel.wal_sync
      { acked := 0#u64, synced := 0#u64, written := 96#u64 }
      env_crash_kernel.SyncHonesty.Lying
      = ok { acked := 0#u64, synced := 0#u64, written := 96#u64 } := by
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-! ## RFC-0191 P2.1 — Inv-WAL preservation (one step) + corollary D1 -/

/-- Forma closed of `inv_wal`: the conjunction Booleana of the two containments
(`acked ⊆ synced` and `synced ⊆ written`; the ceiling of the legal crash is
`written`, therefore `synced ≤ written` is exactly "synced is in the
prefix-recoverable"). -/
theorem wal_inv_closed :
    ∀ (s : wal.wal_state_kernel.WalState),
      wal.wal_state_kernel.inv_wal s =
        ok (((s.acked <= s.synced) : Bool) &&
            ((s.synced <= s.written) : Bool)) := by
  intro s
  unfold wal.wal_state_kernel.inv_wal
  split
  · rename_i h1
    rw [decide_eq_true h1, Bool.true_and]
  · rename_i h1
    rw [decide_eq_false (by simpa using h1), Bool.false_and]

/-- Forma closed of the append ok: writes `written + n`; barrier and
prefix acked do not move. -/
theorem wal_append_closed :
    ∀ (s : wal.wal_state_kernel.WalState) (n w : U64),
      s.written + n = ok w →
        wal.wal_state_kernel.wal_append s n =
          ok { s with written := w } := by
  intro s n w hw
  unfold wal.wal_state_kernel.wal_append
  rw [hw]
  simp only [bind_tc_ok]

/-- Inv-WAL (one step, RFC-0191 P2.1): any `ok` outcome of
`wal_append` preserves `acked ⊆ synced ⊆ prefix-recoverable` — the
barrier and the acked do not move and the log only grows. -/
theorem wal_append_preserves_inv_wal :
    ∀ (s s' : wal.wal_state_kernel.WalState) (n : U64),
      wal.wal_state_kernel.inv_wal s = ok true →
      wal.wal_state_kernel.wal_append s n = ok s' →
        wal.wal_state_kernel.inv_wal s' = ok true := by
  intro s s' n hinv happ
  rw [wal_inv_closed] at hinv
  have hAB := Result.ok.inj hinv
  rw [Bool.and_eq_true] at hAB
  obtain ⟨hA, hB⟩ := hAB
  have hBval : s.synced.val ≤ s.written.val := by
    simpa [decide_eq_true_eq, UScalar.le_equiv] using hB
  cases hadd : s.written + n with
  | ok w =>
    rw [wal_append_closed s n w hadd] at happ
    have hs' : s' = { s with written := w } := (Result.ok.inj happ).symm
    subst hs'
    have hwval : w.val = s.written.val + n.val := by
      have hz := UScalar.add_equiv s.written n
      rw [hadd] at hz
      exact hz.2.1
    have hle : s.synced ≤ w := by
      rw [UScalar.le_equiv]
      omega
    rw [wal_inv_closed]
    simp only []
    rw [hA, Bool.true_and, decide_eq_true hle]
  | fail e =>
    unfold wal.wal_state_kernel.wal_append at happ
    rw [hadd] at happ
    simp at happ
  | div =>
    unfold wal.wal_state_kernel.wal_append at happ
    rw [hadd] at happ
    simp at happ

/-- Corollary D1 (RFC-0191 P2.1): when the plan of the kernel that the rustc
turns on (`wal_commit_plan`) sends Sync before of Apply/Ok (close P1.2), the
step of append that the precede preserves Inv-WAL — the Ok of the client only
there is with `acked ⊆ synced ⊆ prefix-recoverable`. Cites the close P1.2
(`d1_wal_commit_plan`) e o lemma `wal_append_preserves_inv_wal`. -/
theorem d1_plan_append_preserves_inv_wal :
    ∀ (need_sync sync_fail : Bool)
      (s s' : wal.wal_state_kernel.WalState) (n : U64),
      wal_commit_plan need_sync sync_fail
        = ok WalCommitPlan.AppendSyncApplyOk →
      wal.wal_state_kernel.inv_wal s = ok true →
      wal.wal_state_kernel.wal_append s n = ok s' →
        wal.wal_state_kernel.inv_wal s' = ok true := by
  intro need_sync sync_fail s s' n hplan hinv happ
  rw [d1_wal_commit_plan] at hplan
  cases need_sync
  · exact absurd hplan (by intro hh; simp at hh)
  · cases sync_fail
    · exact wal_append_preserves_inv_wal s s' n hinv happ
    · exact absurd hplan (by intro hh; simp at hh)

/-! ## RFC-0198 P1.1 — base initial + reachability inductive (Inv-WAL) -/

/-- State initial of the WAL (log empty): the three watermarks in zero — the
that the production builds for the log new (`wal_state_of 0 0 0`: only there is the
prefix empty, nothing acked, nothing synced). -/
def wal_state_init : wal.wal_state_kernel.WalState :=
  { acked := 0#u64, synced := 0#u64, written := 0#u64 }

/-- BASE of the induction: the log empty satisfaz Inv-WAL (zero ⊆ zero ⊆ zero). -/
theorem inv_wal_init :
    wal.wal_state_kernel.inv_wal wal_state_init = ok true := by
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- Reachability by n appends ok (forma inductive seL4): the state is
reachable when there is the chain of `n` steps `wal_append` that
retornam ok starting from the state initial. -/
inductive wal_append_reach :
    Nat → wal.wal_state_kernel.WalState → Prop
  | zero : wal_append_reach 0 wal_state_init
  | succ (m : Nat) (s s' : wal.wal_state_kernel.WalState) (k : U64) :
      wal_append_reach m s →
      wal.wal_state_kernel.wal_append s k = ok s' →
      wal_append_reach (m + 1) s'

/-- COROLLARY DE REACHABILITY (RFC-0198 P1.1): every state reachable
by n appends satisfaz Inv-WAL. A base is `inv_wal_init`; each step is the
lemma one-step REGISTRADO `wal_append_preserves_inv_wal` (RFC-0191 P2.1)
— the inductive only chains the steps, does not re-prove them. -/
theorem inv_wal_reachable :
    ∀ (n : Nat) (s : wal.wal_state_kernel.WalState),
      wal_append_reach n s →
      wal.wal_state_kernel.inv_wal s = ok true := by
  intro n s hr
  induction hr with
  | zero => exact inv_wal_init
  | @succ m s s' k _hreach happ ih =>
      exact wal_append_preserves_inv_wal s s' k ih happ

/-! ## RFC-0198 P1.2 — step sync/fence preserves Inv-WAL -/

/-- Forma closed of the step of barrier honest (∀ states): the sync
Honest promotes `synced` the `written` — the min of `CrashModel.of` is
discartado by the step itself (only decides if the bind failure, and the two
legs of the min are `ok`). -/
private theorem wal_sync_honest_closed :
    ∀ (s : wal.wal_state_kernel.WalState),
      wal.wal_state_kernel.wal_sync s env_crash_kernel.SyncHonesty.Honest
        = ok { s with synced := s.written, written := s.written } := by
  intro s
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant, bind_tc_ok]
  split_ifs <;> simp_all

/-- Closed form of the lying barrier step (∀ states): the
watermarks stay where they are — `CrashModel.of` only cuts `synced` back by the
min with `written` (never widens), and the Lying sync returns the model
itself. -/
private theorem wal_sync_lying_closed :
    ∀ (s : wal.wal_state_kernel.WalState),
      wal.wal_state_kernel.wal_sync s env_crash_kernel.SyncHonesty.Lying
        = ok { s with
            synced :=
              (if s.synced.val < s.written.val then s.synced else s.written),
            written := s.written } := by
  intro s
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant, bind_tc_ok]
  split_ifs <;> simp_all

/-- Inv-WAL (sync step, RFC-0198 P1.2): any `ok` outcome of
`wal_sync` preserves Inv-WAL, under both Env honesty modes — the honest
sync promotes the barrier up to `written` (never beyond); the lying sync
leaves the watermarks where they are (the min only cuts back, never widens). -/
theorem wal_sync_preserves_inv_wal :
    ∀ (h : env_crash_kernel.SyncHonesty)
      (s s' : wal.wal_state_kernel.WalState),
      wal.wal_state_kernel.inv_wal s = ok true →
      wal.wal_state_kernel.wal_sync s h = ok s' →
        wal.wal_state_kernel.inv_wal s' = ok true := by
  intro h s s' hinv happ
  rw [wal_inv_closed] at hinv
  have hAB := Result.ok.inj hinv
  rw [Bool.and_eq_true] at hAB
  obtain ⟨hA, hB⟩ := hAB
  have hAval : s.acked.val ≤ s.synced.val := by
    simpa [decide_eq_true_eq, UScalar.le_equiv] using hA
  have hBval : s.synced.val ≤ s.written.val := by
    simpa [decide_eq_true_eq, UScalar.le_equiv] using hB
  have finish : ∀ w : U64, s.synced.val ≤ w.val → w.val ≤ s.written.val →
      wal.wal_state_kernel.inv_wal
        { s with synced := w, written := s.written } = ok true := by
    intro w hw1 hw2
    rw [wal_inv_closed]
    have hle1 : s.acked ≤ w := by
      rw [UScalar.le_equiv]; omega
    have hle2 : w ≤ s.written := by
      rw [UScalar.le_equiv]; omega
    simp only [decide_eq_true hle1, decide_eq_true hle2, Bool.true_and]
  cases h with
  | Honest =>
      rw [wal_sync_honest_closed] at happ
      have hs' : s' = { s with synced := s.written, written := s.written } :=
        (Result.ok.inj happ).symm
      rw [hs']
      exact finish s.written hBval (Nat.le_refl _)
  | Lying =>
      rw [wal_sync_lying_closed] at happ
      have hs' : s' = { s with
          synced :=
            (if s.synced.val < s.written.val then s.synced else s.written),
          written := s.written } := (Result.ok.inj happ).symm
      rw [hs']
      split
      · next hlt =>
          exact finish s.synced (Nat.le_refl _) hBval
      · next hlt =>
          exact finish s.written hBval (Nat.le_refl _)

/-- Inv-WAL (ack step, RFC-0198 P1.2): any `ok` outcome of
`wal_ack` preserves Inv-WAL — the ack only advances `acked` when the
saturated value fits in `synced`; when the checked add overflows the step is not even
`ok`, therefore the client Ok never breaks `acked ⊆ synced`. -/
theorem wal_ack_preserves_inv_wal :
    ∀ (s s' : wal.wal_state_kernel.WalState) (n : U64),
      wal.wal_state_kernel.inv_wal s = ok true →
      wal.wal_state_kernel.wal_ack s n = ok s' →
        wal.wal_state_kernel.inv_wal s' = ok true := by
  intro s s' n hinv happ
  rw [wal_inv_closed] at hinv
  have hAB := Result.ok.inj hinv
  rw [Bool.and_eq_true] at hAB
  obtain ⟨hA, hB⟩ := hAB
  unfold wal.wal_state_kernel.wal_ack at happ
  simp only [lift, bind_tc_ok] at happ
  split at happ
  · next hle =>
      -- hle (after split): the containment of the value saturated, already in coercion
      cases hadd : s.acked + n with
      | ok a =>
          rw [hadd] at happ
          simp only [bind_tc_ok] at happ
          have hs' : s' = { s with acked := a } := (Result.ok.inj happ).symm
          subst hs'
          have hz := UScalar.add_equiv s.acked n
          rw [hadd] at hz
          obtain ⟨hbound, hval, _⟩ := hz
          have hleval : (core.num.U64.saturating_add s.acked n).val
              ≤ s.synced.val := by
            simpa [UScalar.le_equiv] using hle
          have hbits : (2 : Nat) ^ UScalarTy.U64.numBits
              = 18446744073709551616 := by native_decide
          rw [hbits] at hbound
          have hsat : (core.num.U64.saturating_add s.acked n).val
              = s.acked.val + n.val := by
            have hmax : ((UScalar.max UScalarTy.U64 : Nat)) + 1
                = 18446744073709551616 := by native_decide
            show (Nat.min ((UScalar.max UScalarTy.U64 : Nat))
                (s.acked.val + n.val)
                % 18446744073709551616) = _
            have hminlt : Nat.min ((UScalar.max UScalarTy.U64 : Nat))
                (s.acked.val + n.val)
                < 18446744073709551616 :=
              Nat.lt_of_le_of_lt (Nat.min_le_left _ _) (by omega)
            rw [Nat.mod_eq_of_lt hminlt]
            exact Nat.min_eq_right (by omega)
          rw [wal_inv_closed]
          have hale : a ≤ s.synced := by
            rw [UScalar.le_equiv]
            omega
          simp only [decide_eq_true hale, Bool.true_and, hB]
      | fail e =>
          rw [hadd] at happ
          simp at happ
      | div =>
          rw [hadd] at happ
          simp at happ
  · next _ =>
      have hs' : s' = s := (Result.ok.inj happ).symm
      subst hs'
      rw [wal_inv_closed]
      exact hinv

/-- Classe of step of the write path that toca the WAL: append, sync or ack. -/
inductive wal_write_step :
    wal.wal_state_kernel.WalState → wal.wal_state_kernel.WalState → Prop
  | append (s s' : wal.wal_state_kernel.WalState) (n : U64) :
      wal.wal_state_kernel.wal_append s n = ok s' → wal_write_step s s'
  | sync (s s' : wal.wal_state_kernel.WalState)
      (h : env_crash_kernel.SyncHonesty) :
      wal.wal_state_kernel.wal_sync s h = ok s' → wal_write_step s s'
  | ack (s s' : wal.wal_state_kernel.WalState) (n : U64) :
      wal.wal_state_kernel.wal_ack s n = ok s' → wal_write_step s s'

/-- Corollary of the classe complete (RFC-0198 P1.2, closes the sentence): TODO
step of the write path that toca the WAL preserves Inv-WAL — each constructor
cites the lemma one-step corresponding (`wal_append_preserves_inv_wal`
RFC-0191 P2.1; `wal_sync_preserves_inv_wal` e `wal_ack_preserves_inv_wal`
P1.2); nothing is re-proved here. -/
theorem wal_write_step_preserves_inv_wal :
    ∀ (s s' : wal.wal_state_kernel.WalState),
      wal.wal_state_kernel.inv_wal s = ok true →
      wal_write_step s s' →
        wal.wal_state_kernel.inv_wal s' = ok true := by
  intro s s' hinv hstep
  cases hstep with
  | append n happ => exact wal_append_preserves_inv_wal _ _ n hinv happ
  | sync h hs => exact wal_sync_preserves_inv_wal h _ _ hinv hs
  | ack n hack => exact wal_ack_preserves_inv_wal _ _ n hinv hack

/-- RFC-0200 P0.1: chain of k steps of the write path starting from the state
initial — ANY constructor of the family (append, sync, ack), in
any order. This is the real physics of group commit, not only
append-accounting. -/
inductive wal_write_step_reach :
    Nat → wal.wal_state_kernel.WalState → Prop
  | init : wal_write_step_reach 0 wal_state_init
  | step (k : Nat) (s s' : wal.wal_state_kernel.WalState) :
      wal_write_step s s' →
      wal_write_step_reach k s →
      wal_write_step_reach (k + 1) s'

/-- RFC-0200 P0.1 COROLLARY: every state reachable by any
sequence of steps of the write path (append/sync/ack intercalados, the
partir of the log empty) satisfaz Inv-WAL — the sentence seL4 complete of the WAL.
Induction over the chain; base = `inv_wal_init` (RFC-0198 P1.1), step =
CITA `wal_write_step_preserves_inv_wal` (RFC-0198 P1.2, registered);
nothing is re-proved here. -/
theorem inv_wal_write_reachable :
    ∀ (k : Nat) (s : wal.wal_state_kernel.WalState),
      wal_write_step_reach k s →
        wal.wal_state_kernel.inv_wal s = ok true := by
  intro k s hreach
  induction hreach with
  | init => exact inv_wal_init
  | step k' s0 s1 hstep _ IH =>
      exact wal_write_step_preserves_inv_wal _ _ IH hstep

/-! ## RFC-0214 P0.1 — durability spine in the atom rung (fate ∀) -/

/-- RFC-0214 P0.1 (atom `catalog:wal_state`): the outcome of `inv_wal`
is EXACTLY the Boolean conjunction of the two containments — `acked ⊆
synced` and `synced ⊆ written` (the ceiling of the legal crash is `written`,
therefore `synced ≤ written` is "synced is in the prefix-recoverable").
Fate forall over the body extracted (pattern `fate_iff` of the RFCs
0205–0213); CITES the closed ∀ `wal_inv_closed` (RFC-0191 P2.1) on the
leg — nothing is re-proved. The mutant AS-IS forgets the
acked⊆synced arm (tooth `inv_wal_as_is_tooth`). -/
theorem inv_wal_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (v : Bool),
      (wal.wal_state_kernel.inv_wal s = ok v) ↔
        (v = (((s.acked <= s.synced) : Bool) &&
              ((s.synced <= s.written) : Bool))) := by
  intro s v
  rw [wal_inv_closed]
  constructor
  · intro h
    exact (Result.ok.inj h).symm
  · intro h
    rw [h]

/-- RFC-0214 P0.1 (atom `catalog:wal_append`): the append has outcome
ok EXACTLY when the byte add does not overflow — and on that match the
single possible future is `{s with written := w}` (barrier and acked
prefix do not move; the log only grows). Fate forall over the body
extracted; the ← route CITES the closed ∀ `wal_append_closed`
(RFC-0191 P2.1). The mutant AS-IS acks the same bytes together with the
write — before any barrier. -/
theorem wal_append_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (n w : U64),
      (wal.wal_state_kernel.wal_append s n = ok { s with written := w }) ↔
        (s.written + n = ok w) := by
  intro s n w
  constructor
  · intro h
    unfold wal.wal_state_kernel.wal_append at h
    cases hadd : s.written + n with
    | ok w' =>
        rw [hadd] at h
        simp only [bind_tc_ok] at h
        have hw : w' = w :=
          congrArg wal.wal_state_kernel.WalState.written (Result.ok.inj h)
        rw [hw]
    | fail e =>
        rw [hadd] at h
        simp at h
    | div =>
        rw [hadd] at h
        simp at h
  · intro h
    exact wal_append_closed s n w h

/-- RFC-0214 P0.1 (atom `catalog:wal_sync`): the sync has outcome ok
with EXACTLY two futures, decided by the Env honesty — Honest
promotes the barrier to `written`; Lying returns the watermarks
recortadas by the min of `CrashModel.of` (never widens). Fate forall
over the body extracted; both the routes CITAM the closed ∀ privados
`wal_sync_honest_closed`/`wal_sync_lying_closed` (RFC-0198 P1.2).
The mutant AS-IS promotes always — even with a lying sync. -/
theorem wal_sync_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState)
      (h : env_crash_kernel.SyncHonesty)
      (s' : wal.wal_state_kernel.WalState),
      (wal.wal_state_kernel.wal_sync s h = ok s') ↔
        ((h = env_crash_kernel.SyncHonesty.Honest ∧
            s' = { s with synced := s.written, written := s.written })
          ∨ (h = env_crash_kernel.SyncHonesty.Lying ∧
            s' = { s with
                synced :=
                  (if s.synced.val < s.written.val then s.synced
                   else s.written),
                written := s.written })) := by
  intro s h s'
  cases h with
  | Honest =>
      rw [wal_sync_honest_closed]
      constructor
      · intro ho
        exact Or.inl ⟨rfl, (Result.ok.inj ho).symm⟩
      · rintro (⟨_, he⟩ | ⟨hf, _⟩)
        · rw [he]
        · exact absurd hf
            (fun hh => env_crash_kernel.SyncHonesty.noConfusion hh)
  | Lying =>
      rw [wal_sync_lying_closed]
      constructor
      · intro ho
        exact Or.inr ⟨rfl, (Result.ok.inj ho).symm⟩
      · rintro (⟨hf, _⟩ | ⟨_, he⟩)
        · exact absurd hf
            (fun hh => env_crash_kernel.SyncHonesty.noConfusion hh)
        · rw [he]

/-- RFC-0214 P0.1 (atom `catalog:wal_ack`): the ack has EXACTLY
two ok futures — inside the barrier (`acked+n = ok a` with the
saturated value contained in `synced`): `{s with acked := a}`; outside it:
refused, the state rolls back whole (`s' = s`). The client Ok never
advances `acked` beyond what the barrier made durable — fail-closed.
Fate forall over the body extracted (the containment is the body's: the
saturated value against `synced`). The mutant AS-IS acks
unconditionally — `acked` passes the barrier. -/
theorem wal_ack_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (n : U64)
      (s' : wal.wal_state_kernel.WalState),
      (wal.wal_state_kernel.wal_ack s n = ok s') ↔
        ((∃ a : U64, core.num.U64.saturating_add s.acked n ≤ s.synced
            ∧ s.acked + n = ok a ∧ s' = { s with acked := a })
          ∨ (¬ core.num.U64.saturating_add s.acked n ≤ s.synced
              ∧ s' = s)) := by
  intro s n s'
  constructor
  · intro h
    unfold wal.wal_state_kernel.wal_ack at h
    simp only [lift, bind_tc_ok] at h
    split at h
    · next hle =>
        cases hadd : s.acked + n with
        | ok a =>
            rw [hadd] at h
            simp only [bind_tc_ok] at h
            refine Or.inl ⟨a, hle, rfl, (Result.ok.inj h).symm⟩
        | fail e =>
            rw [hadd] at h
            simp at h
        | div =>
            rw [hadd] at h
            simp at h
    · next hle =>
        exact Or.inr ⟨hle, (Result.ok.inj h).symm⟩
  · rintro (⟨a, hle, hadd, rfl⟩ | ⟨hle, rfl⟩)
    · unfold wal.wal_state_kernel.wal_ack
      simp only [lift, bind_tc_ok]
      rw [hadd]
      simp only [bind_tc_ok]
      split
      · rfl
      · next hbad => exact absurd hle hbad
    · unfold wal.wal_state_kernel.wal_ack
      simp only [lift, bind_tc_ok]
      split
      · next hbad => exact absurd hbad hle
      · rfl

/-- RFC-0214 P0.1 (atom `catalog:wal_rotate`): the rotate has
EXACTLY two ok futures — the whole log durable and acked
(`acked = synced = written`): the log is zeroed; any non-durable
tail: refused, the state rolls back whole (acked bytes do not
vanish). Fate forall over the body extracted. The mutant AS-IS
derruba the log always — same with non-durable tail. -/
theorem wal_rotate_fate_iff :
    ∀ (s s' : wal.wal_state_kernel.WalState),
      (wal.wal_state_kernel.wal_rotate s = ok s') ↔
        ((s.acked = s.synced ∧ s.synced = s.written ∧
            s' = { acked := 0#u64, synced := 0#u64, written := 0#u64 })
          ∨ (¬(s.acked = s.synced ∧ s.synced = s.written) ∧ s' = s)) := by
  intro s s'
  have hz : wal.wal_state_kernel.wal_state_of 0#u64 0#u64 0#u64
      = ok { acked := 0#u64, synced := 0#u64, written := 0#u64 } := by
    unfold wal.wal_state_kernel.wal_state_of
    simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
      core.cmp.Ord.min_body]
  unfold wal.wal_state_kernel.wal_rotate
  rw [hz]
  split
  · split
    · next h1 h2 =>
        constructor
        · intro ho
          exact Or.inl ⟨h1, h2, (Result.ok.inj ho).symm⟩
        · rintro (⟨_, _, he⟩ | ⟨hn, _⟩)
          · rw [he]
          · exact absurd ⟨h1, h2⟩ hn
    · next h1 h2 =>
        constructor
        · intro ho
          refine Or.inr ⟨?_, (Result.ok.inj ho).symm⟩
          exact fun hc => h2 hc.2
        · rintro (⟨_, hb, _⟩ | ⟨_, he⟩)
          · exact absurd hb h2
          · rw [he]
  · next h1 =>
      constructor
      · intro ho
        refine Or.inr ⟨?_, (Result.ok.inj ho).symm⟩
        exact fun hc => h1 hc.1
      · rintro (⟨ha, _, _⟩ | ⟨_, he⟩)
        · exact absurd ha h1
        · rw [he]

/-- RFC-0214 P0.1 (atom `catalog:wal_acked_survives`): the survival
corollary has outcome ok with EXACTLY two futures, decided
by the Env stitch (`CrashModel.of` + `crash_legal`): legal cut →
`v = (cut ≥ acked)`; illegal cut → `v = true` (nothing to lose). The
acked prefix is only judged LOSABLE by cuts the stitch calls
legal — the legality is the Env's, not re-proved here (P0.2 of the
RFC-0214 will pin `crash_legal`). Fate forall over the body
extracted. The mutant AS-IS calls the cut below the
barrier floor survivable — and loses acked bytes. -/
theorem acked_survives_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (cut : U64) (v : Bool),
      (wal.wal_state_kernel.acked_survives_every_legal_crash s cut
        = ok v) ↔
        (∃ cm : env_crash_kernel.CrashModel,
          ∃ b : Bool,
            env_crash_kernel.CrashModel.of s.written s.synced = ok cm
            ∧ env_crash_kernel.crash_legal cm cut = ok b
            ∧ ((b = true ∧ v = ((cut >= s.acked) : Bool))
                ∨ (b = false ∧ v = true))) := by
  intro s cut v
  constructor
  · intro h
    unfold wal.wal_state_kernel.acked_survives_every_legal_crash at h
    cases hcm : env_crash_kernel.CrashModel.of s.written s.synced with
    | ok cm =>
        rw [hcm] at h
        simp only [bind_tc_ok] at h
        rw [← hcm]
        cases hb : env_crash_kernel.crash_legal cm cut with
        | ok b =>
            rw [hb] at h
            simp only [bind_tc_ok] at h
            split at h
            · next hbT =>
                exact ⟨cm, b, hcm, hb,
                  Or.inl ⟨hbT, (Result.ok.inj h).symm⟩⟩
            · next hbF =>
                refine ⟨cm, b, hcm, hb,
                  Or.inr ⟨?_, (Result.ok.inj h).symm⟩⟩
                cases b with
                | false => rfl
                | true => exact absurd rfl hbF
        | fail e =>
            rw [hb] at h
            simp at h
        | div =>
            rw [hb] at h
            simp at h
    | fail e =>
        rw [hcm] at h
        simp at h
    | div =>
        rw [hcm] at h
        simp at h
  · rintro ⟨cm, b, hcm, hb, hb' | hb'⟩
    · obtain ⟨rfl, hv⟩ := hb'
      unfold wal.wal_state_kernel.acked_survives_every_legal_crash
      rw [hcm]
      simp only [bind_tc_ok]
      rw [hb]
      simp only [bind_tc_ok]
      rw [hv]
      split
      · rfl
      · next hbad => exact False.elim (hbad trivial)
    · obtain ⟨rfl, hv⟩ := hb'
      unfold wal.wal_state_kernel.acked_survives_every_legal_crash
      rw [hcm]
      simp only [bind_tc_ok]
      rw [hb]
      simp only [bind_tc_ok]
      split
      · next hbad => exact Bool.noConfusion hbad
      · rw [hv]
