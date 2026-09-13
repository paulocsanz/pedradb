-- Theorems over Aeneas extract of env_crash_kernel.rs (RFC-0166 P1.1).
import Aeneas
import EnvCrashKernel
open Aeneas.Std Result
open pedra_aeneas_env_crash_kernel

/-- Catalog entry: a cut between the barrier and written is legal. -/
theorem crash_legal_in_window :
    env_crash_kernel.crash_legal
      { written := 10#u64, synced := 4#u64 }
      (7#u64) = ok true := by
  unfold env_crash_kernel.crash_legal
  rfl

/-- AS-IS tooth: a cut below the barrier still admits. -/
theorem crash_legal_as_is_tooth :
    env_crash_kernel.crash_legal_as_is
      { written := 10#u64, synced := 4#u64 }
      (3#u64) = ok true := by
  unfold env_crash_kernel.crash_legal_as_is
  rfl

/-- Honest barrier: `sync` unfolds `fsync_promotes_pending` and promotes. -/
theorem sync_honest_promotes_via_fsync :
    env_crash_kernel.sync
      { written := 96#u64, synced := 0#u64 }
      env_crash_kernel.SyncHonesty.Honest
      = ok { written := 96#u64, synced := 96#u64 } := by
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [env_crash_kernel.SyncHonesty.read_discriminant]

/-- Lying OS: same callee returns false, watermark stays. -/
theorem sync_lying_does_not_promote :
    env_crash_kernel.sync
      { written := 96#u64, synced := 0#u64 }
      env_crash_kernel.SyncHonesty.Lying
      = ok { written := 96#u64, synced := 0#u64 } := by
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [env_crash_kernel.SyncHonesty.read_discriminant]

/-! ## RFC-0214 P0.2 — stitch Env in the atom rung (fate ∀) -/

/-- RFC-0214 P0.2 (atom `catalog:env_crash`): the cut is legal
EXACTLY when survives between the floor of the barrier and the ceiling
written — `synced ⊆ cut ⊆ written` (caudas tornadas can keep
prefix; bytes synced never vanish; none byte is invented).
Fate forall over the body extracted. The mutant AS-IS ignora the floor
of the barrier — the cut below `synced` is called legal and
consumes bytes the barrier promised. -/
theorem crash_legal_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel) (cut : U64) (v : Bool),
      (env_crash_kernel.crash_legal m cut = ok v) ↔
        (v = (((m.synced <= cut) : Bool) &&
              ((cut <= m.written) : Bool))) := by
  intro m cut v
  unfold env_crash_kernel.crash_legal
  split
  · next hle =>
      rw [decide_eq_true hle, Bool.true_and]
      constructor
      · intro h
        exact (Result.ok.inj h).symm
      · intro h
        rw [h]
  · next hgt =>
      rw [decide_eq_false (by simpa [UScalar.le_equiv] using hgt),
        Bool.false_and]
      constructor
      · intro h
        exact (Result.ok.inj h).symm
      · intro h
        rw [h]

/-- RFC-0214 P0.2 (atom `catalog:env_append`): the Env append has
outcome ok EXACTLY when the byte add does not overflow — and
on that match the single future is `{m with written := w}` (the barrier
does not move; the logical length only grows). Fate forall over the
body extracted. The mutant AS-IS of the stitch is the `crash_legal` one
(without floor) — the real append has no mutant of its own in the pair. -/
theorem append_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel) (n w : U64),
      (env_crash_kernel.append m n = ok { m with written := w }) ↔
        (m.written + n = ok w) := by
  intro m n w
  constructor
  · intro h
    unfold env_crash_kernel.append at h
    cases hadd : m.written + n with
    | ok w' =>
        rw [hadd] at h
        simp only [bind_tc_ok] at h
        have hw : w' = w :=
          congrArg env_crash_kernel.CrashModel.written (Result.ok.inj h)
        rw [hw]
    | fail e =>
        rw [hadd] at h
        simp at h
    | div =>
        rw [hadd] at h
        simp at h
  · intro h
    unfold env_crash_kernel.append
    rw [h]
    simp only [bind_tc_ok]

/-- RFC-0214 P0.2 (atom `catalog:env_sync`): the Env sync has
outcome ok with EXACTLY two futures, decided by honesty —
Honest promotes the barrier to the length every; Lying returns Ok and
the model rolls back whole (the barrier does not lie — RFC-0078). Fate
forall over the body extracted. The mutant AS-IS promotes always —
the lying sync is treated as the barrier done. -/
theorem sync_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel)
      (h : env_crash_kernel.SyncHonesty)
      (m' : env_crash_kernel.CrashModel),
      (env_crash_kernel.sync m h = ok m') ↔
        ((h = env_crash_kernel.SyncHonesty.Honest ∧
            m' = { m with synced := m.written })
          ∨ (h = env_crash_kernel.SyncHonesty.Lying ∧ m' = m)) := by
  intro m h m'
  have hc : env_crash_kernel.sync m env_crash_kernel.SyncHonesty.Honest
      = ok { m with synced := m.written } := by
    unfold env_crash_kernel.sync
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
    unfold group_commit_kernel.fsync_promotes_pending
    simp [env_crash_kernel.SyncHonesty.read_discriminant]
  have hl : env_crash_kernel.sync m env_crash_kernel.SyncHonesty.Lying
      = ok m := by
    unfold env_crash_kernel.sync
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
    unfold group_commit_kernel.fsync_promotes_pending
    simp [env_crash_kernel.SyncHonesty.read_discriminant]
  cases h with
  | Honest =>
      rw [hc]
      constructor
      · intro ho
        exact Or.inl ⟨rfl, (Result.ok.inj ho).symm⟩
      · rintro (⟨_, he⟩ | ⟨hf, _⟩)
        · rw [he]
        · exact absurd hf
            (fun hh => env_crash_kernel.SyncHonesty.noConfusion hh)
  | Lying =>
      rw [hl]
      constructor
      · intro ho
        exact Or.inr ⟨rfl, (Result.ok.inj ho).symm⟩
      · rintro (⟨hf, _⟩ | ⟨_, he⟩)
        · exact absurd hf
            (fun hh => env_crash_kernel.SyncHonesty.noConfusion hh)
        · rw [he]

/-- RFC-0214 P0.2 (atom `catalog:env_barrier_floor`): the corollary
of the floor holds ALWAYS — the outcome is `ok v` with `v = true` exact: or the
the cut is illegal (nothing the lose), or is legal and then `cut ≥
synced` (the floor of the window of the `crash_legal_fate_iff`, atom 1/6
of this fatia). Um crash legal never loses byte that the barrier
honest made durable. Fate forall over the body extracted; CITA
`crash_legal_fate_iff`. The mutant AS-IS is the `crash_legal` without
floor — the same tooth of the entry 1/6. -/
theorem barrier_floor_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel) (cut : U64) (v : Bool),
      (env_crash_kernel.barrier_floor_holds m cut = ok v) ↔
        (v = true) := by
  intro m cut v
  have key : env_crash_kernel.barrier_floor_holds m cut = ok true := by
    unfold env_crash_kernel.barrier_floor_holds
    cases hb : env_crash_kernel.crash_legal m cut with
    | ok b =>
        simp only [bind_tc_ok]
        cases b with
        | true =>
            have hwin := ((crash_legal_fate_iff m cut true).mp hb).symm
            rw [Bool.and_eq_true] at hwin
            obtain ⟨hle, _⟩ := hwin
            simp
            exact decide_eq_true_eq.mp hle
        | false => simp
    | fail e =>
        have hf := (crash_legal_fate_iff m cut
            (((m.synced <= cut) : Bool) &&
             ((cut <= m.written) : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
    | div =>
        have hf := (crash_legal_fate_iff m cut
            (((m.synced <= cut) : Bool) &&
             ((cut <= m.written) : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
  rw [key]
  constructor
  · intro h
    exact (Result.ok.inj h).symm
  · intro h
    rw [h]

/-- RFC-0214 P0.2 (atom `catalog:env_no_invented`): the corollary of the
ceiling holds ALWAYS — the outcome is `ok v` with `v = true` exact: or the
cut is illegal, or is legal and then `cut ≤ written` (the ceiling of the
window of the `crash_legal_fate_iff`). The recovery never observa the
byte that the writer did not write. Fate forall over the body
extracted; CITA `crash_legal_fate_iff`. The mutant AS-IS is the
`crash_legal` without floor. -/
theorem no_invented_bytes_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel) (cut : U64) (v : Bool),
      (env_crash_kernel.no_invented_bytes_holds m cut = ok v) ↔
        (v = true) := by
  intro m cut v
  have key : env_crash_kernel.no_invented_bytes_holds m cut
      = ok true := by
    unfold env_crash_kernel.no_invented_bytes_holds
    cases hb : env_crash_kernel.crash_legal m cut with
    | ok b =>
        simp only [bind_tc_ok]
        cases b with
        | true =>
            have hwin := ((crash_legal_fate_iff m cut true).mp hb).symm
            rw [Bool.and_eq_true] at hwin
            obtain ⟨_, hwe⟩ := hwin
            simp
            exact decide_eq_true_eq.mp hwe
        | false => simp
    | fail e =>
        have hf := (crash_legal_fate_iff m cut
            (((m.synced <= cut) : Bool) &&
             ((cut <= m.written) : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
    | div =>
        have hf := (crash_legal_fate_iff m cut
            (((m.synced <= cut) : Bool) &&
             ((cut <= m.written) : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
  rw [key]
  constructor
  · intro h
    exact (Result.ok.inj h).symm
  · intro h
    rw [h]

/-- RFC-0214 P0.2 (atom `catalog:env_honest_sync`): the corollary of the
sync honest holds ALWAYS — the outcome is `ok v` with `v = true`
exact: after the barrier honest (`synced := written`), the window
legal collapses to a single point (`written ≤ cut ≤ written` forces
`cut = written`, legs `sync_fate_iff` + `crash_legal_fate_iff`,
atoms 3/6 and 1/6 of this slice) — every legal crash preserves the log
WHOLE. Fate forall over the body extracted. The mutant AS-IS
promotes a lying sync — the promised barrier does not exist. -/
theorem honest_sync_fate_iff :
    ∀ (m : env_crash_kernel.CrashModel) (cut : U64) (v : Bool),
      (env_crash_kernel.honest_sync_protects_all m cut = ok v) ↔
        (v = true) := by
  intro m cut v
  have hs : env_crash_kernel.sync m env_crash_kernel.SyncHonesty.Honest
      = ok { m with synced := m.written } := by
    unfold env_crash_kernel.sync
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
    unfold group_commit_kernel.fsync_promotes_pending
    simp [env_crash_kernel.SyncHonesty.read_discriminant]
  have key : env_crash_kernel.honest_sync_protects_all m cut
      = ok true := by
    unfold env_crash_kernel.honest_sync_protects_all
    rw [hs]
    simp only [bind_tc_ok]
    cases hb : env_crash_kernel.crash_legal
        { m with synced := m.written } cut with
    | ok b =>
        simp only [bind_tc_ok]
        cases b with
        | true =>
            have hwin := ((crash_legal_fate_iff
              { m with synced := m.written } cut true).mp hb).symm
            rw [Bool.and_eq_true] at hwin
            obtain ⟨hle, hwe⟩ := hwin
            have h1 : m.written <= cut := of_decide_eq_true hle
            have h2 : cut <= m.written := of_decide_eq_true hwe
            have heq : cut = m.written := le_antisymm h2 h1
            simp [heq]
        | false => simp
    | fail e =>
        have hf := (crash_legal_fate_iff
            { m with synced := m.written } cut
            (((({ m with synced := m.written } : env_crash_kernel.CrashModel).synced
                <= cut) : Bool) &&
             ((cut <=
                ({ m with synced := m.written } : env_crash_kernel.CrashModel).written)
               : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
    | div =>
        have hf := (crash_legal_fate_iff
            { m with synced := m.written } cut
            (((({ m with synced := m.written } : env_crash_kernel.CrashModel).synced
                <= cut) : Bool) &&
             ((cut <=
                ({ m with synced := m.written } : env_crash_kernel.CrashModel).written)
               : Bool))).mpr rfl
        rw [hb] at hf
        simp at hf
  rw [key]
  constructor
  · intro h
    exact (Result.ok.inj h).symm
  · intro h
    rw [h]
