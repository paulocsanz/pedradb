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

/-- AS-IS dente: a cut below the barrier still admits. -/
theorem crash_legal_as_is_dente :
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

/-! ## RFC-0214 P0.2 — costura Env no degrau átomo (fate ∀) -/

/-- RFC-0214 P0.2 (atom `catalog:env_crash`): um corte é legal
EXATAMENTE quando sobrevive entre o piso da barreira e o teto
escrito — `synced ⊆ cut ⊆ written` (caudas tornadas podem manter
prefixo; bytes synced nunca somem; nenhum byte é inventado).
Fate forall sobre o corpo extraído. O mutante AS-IS ignora o piso
da barreira — um corte abaixo de `synced` é chamado de legal e
come bytes que a barreira prometeu. -/
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
