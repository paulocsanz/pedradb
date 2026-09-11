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

/-! ## RFC-0198 P1.1 — base inicial + alcançabilidade indutiva (Inv-WAL) -/

/-- Estado inicial do WAL (log vazio): os três watermarks em zero — o
que a produção constrói para um log novo (`wal_state_of 0 0 0`: só há o
prefixo vazio, nada acked, nada synced). -/
def wal_state_init : wal.wal_state_kernel.WalState :=
  { acked := 0#u64, synced := 0#u64, written := 0#u64 }

/-- BASE da indução: o log vazio satisfaz Inv-WAL (zero ⊆ zero ⊆ zero). -/
theorem inv_wal_init :
    wal.wal_state_kernel.inv_wal wal_state_init = ok true := by
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- Alcançabilidade por n appends ok (forma indutiva seL4): um estado é
alcançável quando existe uma cadeia de `n` passos `wal_append` que
retornam ok a partir do estado inicial. -/
inductive wal_append_reach :
    Nat → wal.wal_state_kernel.WalState → Prop
  | zero : wal_append_reach 0 wal_state_init
  | succ (m : Nat) (s s' : wal.wal_state_kernel.WalState) (k : U64) :
      wal_append_reach m s →
      wal.wal_state_kernel.wal_append s k = ok s' →
      wal_append_reach (m + 1) s'

/-- COROLÁRIO DE ALCANÇABILIDADE (RFC-0198 P1.1): todo estado alcançável
por n appends satisfaz Inv-WAL. A base é `inv_wal_init`; cada passo é o
lema um-passo REGISTRADO `wal_append_preserves_inv_wal` (RFC-0191 P2.1)
— o indutivo apenas encadeia os passos, não os re-prova. -/
theorem inv_wal_reachable :
    ∀ (n : Nat) (s : wal.wal_state_kernel.WalState),
      wal_append_reach n s →
      wal.wal_state_kernel.inv_wal s = ok true := by
  intro n s hr
  induction hr with
  | zero => exact inv_wal_init
  | @succ m s s' k _hreach happ ih =>
      exact wal_append_preserves_inv_wal s s' k ih happ

/-! ## RFC-0198 P1.2 — passo sync/fence preserva Inv-WAL -/

/-- Forma fechada do passo de barreira honesto (∀ estados): o sync
Honest promove `synced` a `written` — o min de `CrashModel.of` é
discartado pelo próprio passo (só decide se o bind falha, e as duas
pernas do min são `ok`). -/
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

/-- Forma fechada do passo de barreira mentiroso (∀ estados): as
watermarks ficam onde estão — `CrashModel.of` só recorta `synced` pelo
min com `written` (nunca amplia), e o sync Lying devolve o próprio
modelo. -/
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

/-- Inv-WAL (passo sync, RFC-0198 P1.2): qualquer desfecho `ok` de
`wal_sync` preserva Inv-WAL, nas duas honestidades do Env — o sync
honest promove a barreira até `written` (nunca além); o sync mentiroso
deixa as watermarks onde estão (o min só recorta, nunca amplia). -/
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

/-- Inv-WAL (passo ack, RFC-0198 P1.2): qualquer desfecho `ok` de
`wal_ack` preserva Inv-WAL — o ack só avança `acked` quando o valor
saturado cabe em `synced`; quando o add checado estoura o passo nem é
`ok`, logo o Ok do cliente nunca quebra `acked ⊆ synced`. -/
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
      -- hle (após split): a contenção do valor saturado, já em coerção
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

/-- Classe de passo do write path que toca o WAL: append, sync ou ack. -/
inductive wal_write_step :
    wal.wal_state_kernel.WalState → wal.wal_state_kernel.WalState → Prop
  | append (s s' : wal.wal_state_kernel.WalState) (n : U64) :
      wal.wal_state_kernel.wal_append s n = ok s' → wal_write_step s s'
  | sync (s s' : wal.wal_state_kernel.WalState)
      (h : env_crash_kernel.SyncHonesty) :
      wal.wal_state_kernel.wal_sync s h = ok s' → wal_write_step s s'
  | ack (s s' : wal.wal_state_kernel.WalState) (n : U64) :
      wal.wal_state_kernel.wal_ack s n = ok s' → wal_write_step s s'

/-- Corolário da classe completa (RFC-0198 P1.2, fecha a frase): TODO
passo do write path que toca o WAL preserva Inv-WAL — cada construtor
cita o lema um-passo correspondente (`wal_append_preserves_inv_wal`
RFC-0191 P2.1; `wal_sync_preserves_inv_wal` e `wal_ack_preserves_inv_wal`
P1.2); nada é re-provado aqui. -/
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
