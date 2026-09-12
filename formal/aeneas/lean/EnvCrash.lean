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

/-- RFC-0214 P0.2 (atom `catalog:env_append`): o append do Env tem
desfecho ok EXATAMENTE quando a soma dos bytes não estoura — e
nesse caso o único futuro é `{m with written := w}` (a barreira
não se move; o comprimento lógico só cresce). Fate forall sobre o
corpo extraído. O mutante AS-IS da costura é o do `crash_legal`
(sem piso) — o append real não tem mutant próprio no par. -/
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

/-- RFC-0214 P0.2 (atom `catalog:env_sync`): o sync do Env tem
desfecho ok com EXATAMENTE dois futuros, um por honestidade —
Honest promove a barreira ao comprimento todo; Lying devolve Ok e
o modelo volta inteiro (a barreira não mente — RFC-0078). Fate
forall sobre o corpo extraído. O mutante AS-IS promove sempre —
um sync mentiroso é tratado como barreira feita. -/
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

/-- RFC-0214 P0.2 (atom `catalog:env_barrier_floor`): a corolária
do piso vale SEMPRE — o desfecho é `ok v` com `v = true` exato: ou
o corte é ilegal (nada a perder), ou é legal e então `cut ≥
synced` (o piso da janela do `crash_legal_fate_iff`, átomo 1/6
desta fatia). Um crash legal nunca perde byte que a barreira
honesta tornou durável. Fate forall sobre o corpo extraído; CITA
`crash_legal_fate_iff`. O mutante AS-IS é o `crash_legal` sem
piso — o mesmo dente da entrada 1/6. -/
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

/-- RFC-0214 P0.2 (atom `catalog:env_no_invented`): a corolária do
teto vale SEMPRE — o desfecho é `ok v` com `v = true` exato: ou o
corte é ilegal, ou é legal e então `cut ≤ written` (o teto da
janela do `crash_legal_fate_iff`). A recuperação nunca observa um
byte que o writer não escreveu. Fate forall sobre o corpo
extraído; CITA `crash_legal_fate_iff`. O mutante AS-IS é o
`crash_legal` sem piso. -/
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
