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

/-- AS-IS dente: acked past the barrier still admits. -/
theorem inv_wal_as_is_dente :
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

/-- AS-IS dente: a cut below the barrier is treated as legal and fails. -/
theorem acked_survives_as_is_dente :
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

/-! ## RFC-0191 P2.1 — Inv-WAL preservação (um passo) + corolário D1 -/

/-- Forma fechada de `inv_wal`: a conjunção Booleana das duas contenções
(`acked ⊆ synced` e `synced ⊆ written`; o teto de um crash legal é
`written`, logo `synced ≤ written` é exatamente "synced está no
prefixo-recuperável"). -/
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

/-- Forma fechada de um append ok: escreve `written + n`; barreira e
prefixo acked não se movem. -/
theorem wal_append_closed :
    ∀ (s : wal.wal_state_kernel.WalState) (n w : U64),
      s.written + n = ok w →
        wal.wal_state_kernel.wal_append s n =
          ok { s with written := w } := by
  intro s n w hw
  unfold wal.wal_state_kernel.wal_append
  rw [hw]
  simp only [bind_tc_ok]

/-- Inv-WAL (um passo, RFC-0191 P2.1): qualquer desfecho `ok` de
`wal_append` preserva `acked ⊆ synced ⊆ prefixo-recuperável` — a
barreira e o acked não se movem e o log só cresce. -/
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

/-- Corolário D1 (RFC-0191 P2.1): quando o plano do kernel que o rustc
liga (`wal_commit_plan`) manda Sync antes de Apply/Ok (close P1.2), o
passo de append que o precede preserva Inv-WAL — o Ok do cliente só
existe com `acked ⊆ synced ⊆ prefixo-recuperável`. Cita o close P1.2
(`d1_wal_commit_plan`) e o lema `wal_append_preserves_inv_wal`. -/
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
