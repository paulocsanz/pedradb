-- Theorems over Aeneas extract of write_ack_kernel.rs (RFC-0166 P1.4).
import Aeneas
import WriteAckKernel
open Aeneas.Std Result
open pedra_aeneas_write_ack_kernel

/-- Catalog entry: on_append grows written, not the barrier. -/
theorem on_append_grows_written :
    write_ack_kernel.WriteAckLedger.on_append
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      (96#u64) =
      ok { state := { acked := 0#u64, synced := 0#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_append
  unfold wal.wal_state_kernel.wal_append
  rfl

/-- AS-IS dente: ack without a barrier (acked past synced). -/
theorem write_ack_ledger_as_is_dente :
    write_ack_kernel.write_ack_ledger_as_is
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      (96#u64) =
      ok { state := { acked := 96#u64, synced := 0#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.write_ack_ledger_as_is
  unfold write_ack_kernel.WriteAckLedger.on_append
  unfold wal.wal_state_kernel.wal_append
  unfold wal.wal_state_kernel.wal_ack_as_is
  rfl

/-- Honest barrier: `on_barrier` unfolds Honest `wal_sync` + `fsync_promotes_pending`. -/
theorem on_barrier_honest_promotes :
    write_ack_kernel.WriteAckLedger.on_barrier
      { state := { acked := 0#u64, synced := 0#u64, written := 96#u64 } }
      = ok { state := { acked := 0#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_barrier
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-- Other possibility of the same caller: empty ledger, Honest promote is a no-op.
    Production `on_barrier` hardcodes Honest; there is no Lying/`on_barrier_as_is`.
    The as-is ledger path that skips the barrier is `write_ack_ledger_as_is_dente`. -/
theorem on_barrier_empty_is_id :
    write_ack_kernel.WriteAckLedger.on_barrier
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      = ok { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_barrier
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-- Publish: `on_ack` unfolds `wal_ack` of the synced-acked gap. -/
theorem on_ack_promotes_acked :
    write_ack_kernel.WriteAckLedger.on_ack
      { state := { acked := 0#u64, synced := 96#u64, written := 96#u64 } }
      = ok { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_ack
  have hsub : (96#u64 - 0#u64) = ok (96#u64) := rfl
  have hadd : (0#u64 + 96#u64) = ok (96#u64) := rfl
  have hsat : core.num.U64.saturating_add (0#u64) (96#u64) = 96#u64 := by
    native_decide
  have hack :
      wal.wal_state_kernel.wal_ack
        { acked := 0#u64, synced := 96#u64, written := 96#u64 } (96#u64)
        = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_ack
    simp [lift, hsat, hadd]
  simp [hsub, hack]

/-- Other branch of the same caller: already caught up, `wal_ack` of 0 is identity. -/
theorem on_ack_already_caught_up :
    write_ack_kernel.WriteAckLedger.on_ack
      { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } }
      = ok { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_ack
  have hsub : (96#u64 - 96#u64) = ok (0#u64) := rfl
  have hadd : (96#u64 + 0#u64) = ok (96#u64) := rfl
  have hsat : core.num.U64.saturating_add (96#u64) (0#u64) = 96#u64 := by
    native_decide
  have hack :
      wal.wal_state_kernel.wal_ack
        { acked := 96#u64, synced := 96#u64, written := 96#u64 } (0#u64)
        = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_ack
    simp [lift, hsat, hadd]
  simp [hsub, hack]

/-- Fail-closed Inv-WAL: `assert_inv` unfolds `inv_wal`. -/
theorem assert_inv_well_formed :
    write_ack_kernel.WriteAckLedger.assert_inv
      { state := { acked := 0#u64, synced := 4#u64, written := 10#u64 } }
      = ok () := by
  unfold write_ack_kernel.WriteAckLedger.assert_inv
  unfold wal.wal_state_kernel.inv_wal
  simp [massert]

/-- Other branch: ill-formed ledger fails the assertion. -/
theorem assert_inv_ill_formed_fails :
    write_ack_kernel.WriteAckLedger.assert_inv
      { state := { acked := 5#u64, synced := 0#u64, written := 10#u64 } }
      = fail Error.assertionFailure := by
  unfold write_ack_kernel.WriteAckLedger.assert_inv
  unfold wal.wal_state_kernel.inv_wal
  simp [massert]

/-- D1 cut inside the window: `call_mut` unfolds `d1_modelo`. -/
theorem d1_holds_cut_in_window :
    write_ack_kernel.WriteAckLedger.d1_holds_every_cut.closure.Insts.CoreOpsFunctionFnMutTupleU64Bool.call_mut
      { state := { acked := 4#u64, synced := 4#u64, written := 10#u64 } }
      (7#u64)
      = ok (true, { state := { acked := 4#u64, synced := 4#u64, written := 10#u64 } }) := by
  unfold write_ack_kernel.WriteAckLedger.d1_holds_every_cut.closure.Insts.CoreOpsFunctionFnMutTupleU64Bool.call_mut
  unfold d1_modelo_kernel.d1_modelo
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS dente: a cut below the barrier is treated as legal and the corollary fails. -/
theorem d1_holds_as_is_cut_below_barrier :
    d1_modelo_kernel.d1_modelo_as_is
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (4#u64) (3#u64) = ok false := by
  unfold d1_modelo_kernel.d1_modelo_as_is
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-! ## RFC-0214 P1.1 — costura WriteAck no degrau átomo (fate ∀) -/

/-- RFC-0214 P1.1 (atom `catalog:write_ack_append`): o passo append
do ledger é o átomo `wal_append` — `written` cresce por `bytes`,
`acked`/`synced` intocados (o append nunca fabrica barreira nem
ack). Fate forall sobre o corpo extraído. O mutante AS-IS
(`write_ack_ledger_as_is`) acka sem barreira — a planta DST
`verified_write_ack_on_live_profile_is_not_ok` recusa. -/
theorem on_append_fate_iff :
    ∀ (l : write_ack_kernel.WriteAckLedger) (bytes w : U64),
      (write_ack_kernel.WriteAckLedger.on_append l bytes
        = ok { l with state := { l.state with written := w } }) ↔
          (l.state.written + bytes = ok w) := by
  intro l bytes w
  constructor
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_append at h
    unfold wal.wal_state_kernel.wal_append at h
    cases hadd : l.state.written + bytes with
    | ok w' =>
        rw [hadd] at h
        simp only [bind_tc_ok] at h
        have hw : w' = w :=
          congrArg (fun s => s.state.written) (Result.ok.inj h)
        rw [hw]
    | fail e =>
        rw [hadd] at h
        simp at h
    | div =>
        rw [hadd] at h
        simp at h
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_append
    unfold wal.wal_state_kernel.wal_append
    rw [h]
    simp only [bind_tc_ok]

/-- RFC-0214 P1.1 (atom `catalog:write_ack_barrier`): o passo barrier
do ledger é o átomo `wal_sync` Honest — o Ok é EXATAMENTE o estado
promovido (`synced := written` via `fsync_promotes_pending`; o min de
`CrashModel.of` é sobrescrito pelo ramo Honest): não existe Ok que
deixe `synced` atrás de `written`. Fate forall sobre o corpo extraído.
O mutante AS-IS que pula a barreira (`write_ack_ledger_as_is`) é
recusado pela planta DST `verified_write_ack_on_live_profile_is_not_ok`. -/
theorem on_barrier_fate_iff :
    ∀ (l l' : write_ack_kernel.WriteAckLedger),
      (write_ack_kernel.WriteAckLedger.on_barrier l = ok l') ↔
        (l' = { l with state := { l.state with synced := l.state.written, written := l.state.written } }) := by
  intro l l'
  constructor
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_barrier at h
    unfold wal.wal_state_kernel.wal_sync at h
    unfold env_crash_kernel.CrashModel.of at h
    unfold env_crash_kernel.sync at h
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq at h
    unfold group_commit_kernel.fsync_promotes_pending at h
    simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
      core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
      env_crash_kernel.SyncHonesty.read_discriminant] at h
    split at h <;> simp only [bind_tc_ok] at h <;> exact (Result.ok.inj h).symm
  · intro h
    subst h
    unfold write_ack_kernel.WriteAckLedger.on_barrier
    unfold wal.wal_state_kernel.wal_sync
    unfold env_crash_kernel.CrashModel.of
    unfold env_crash_kernel.sync
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
    unfold group_commit_kernel.fsync_promotes_pending
    simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
      core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
      env_crash_kernel.SyncHonesty.read_discriminant]
    split <;> simp only [bind_tc_ok]

/-- RFC-0214 P1.1 (atom `catalog:write_ack_ack`): o passo ack do
ledger é o átomo `wal_ack` sobre o gap synced−acked — o Ok existe
iff `acked ≤ synced` (o passo exige a invariante) e é EXATAMENTE o
estado com `acked := synced`: o gap vira acknowledged, nada além
(o saturado do corpo promove só até a barreira). Fora da
invariante a subtração checada do gap falha — sem Ok, o ack nunca
fabrica durabilidade. Fate forall sobre o corpo extraído. O
mutante AS-IS (`write_ack_ledger_as_is`) acka sem barreira —
recusado pela planta DST
`verified_write_ack_on_live_profile_is_not_ok`. -/
theorem on_ack_fate_iff :
    ∀ (l l' : write_ack_kernel.WriteAckLedger),
      (write_ack_kernel.WriteAckLedger.on_ack l = ok l') ↔
        (l.state.acked ≤ l.state.synced ∧
          l' = { l with state := { l.state with acked := l.state.synced } }) := by
  intro l l'
  constructor
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_ack at h
    cases hsub : l.state.synced - l.state.acked with
    | ok pending =>
        rw [hsub] at h
        simp only [bind_tc_ok] at h
        have hz := UScalar.sub_equiv l.state.synced l.state.acked
        rw [hsub] at hz
        obtain ⟨hAleS, hSval, _⟩ := hz
        have hbits : (2 : Nat) ^ UScalarTy.U64.numBits
            = 18446744073709551616 := by native_decide
        have hmax : ((UScalar.max UScalarTy.U64 : Nat)) + 1
            = 18446744073709551616 := by native_decide
        have hbound : l.state.synced.val < 18446744073709551616 :=
          U64.lt_succ_max l.state.synced
        have hle : l.state.acked ≤ l.state.synced := by
          rw [UScalar.le_equiv]
          omega
        have hmp : l.state.acked.val + pending.val = l.state.synced.val := by
          omega
        have hval : (core.num.U64.saturating_add l.state.acked pending).val
            = l.state.synced.val := by
          show (Nat.min ((UScalar.max UScalarTy.U64 : Nat))
              (l.state.acked.val + pending.val)
              % 18446744073709551616) = _
          have hminlt : Nat.min ((UScalar.max UScalarTy.U64 : Nat))
              (l.state.acked.val + pending.val)
              < 18446744073709551616 :=
            Nat.lt_of_le_of_lt (Nat.min_le_left _ _) (by omega)
          rw [Nat.mod_eq_of_lt hminlt]
          rw [hmp]
          exact Nat.min_eq_right (by omega)
        have hsat : core.num.U64.saturating_add l.state.acked pending
            = l.state.synced := UScalar.eq_imp _ _ hval
        have hadd : l.state.acked + pending = ok l.state.synced := by
          have hza := UScalar.add_equiv l.state.acked pending
          cases haddc : l.state.acked + pending with
          | ok a =>
              rw [haddc] at hza
              obtain ⟨_, haval, _⟩ := hza
              exact congrArg ok (UScalar.eq_imp a l.state.synced (by rw [haval, hmp]))
          | fail e =>
              rw [haddc] at hza
              exfalso
              rw [hmp, hbits] at hza
              omega
          | div =>
              rw [haddc] at hza
              cases hza
        unfold wal.wal_state_kernel.wal_ack at h
        simp only [lift, bind_tc_ok] at h
        rw [hsat, hadd] at h
        simp only [bind_tc_ok] at h
        split at h
        · exact ⟨hle, (Result.ok.inj h).symm⟩
        · next hbad =>
            exact absurd (by simp [UScalar.le_equiv]) hbad
    | fail e =>
        rw [hsub] at h
        simp at h
    | div =>
        rw [hsub] at h
        simp at h
  · rintro ⟨hle, hl'⟩
    subst hl'
    unfold write_ack_kernel.WriteAckLedger.on_ack
    cases hsub : l.state.synced - l.state.acked with
    | ok pending =>
        have hz := UScalar.sub_equiv l.state.synced l.state.acked
        rw [hsub] at hz
        obtain ⟨hAleS, hSval, _⟩ := hz
        have hbits : (2 : Nat) ^ UScalarTy.U64.numBits
            = 18446744073709551616 := by native_decide
        have hmax : ((UScalar.max UScalarTy.U64 : Nat)) + 1
            = 18446744073709551616 := by native_decide
        have hbound : l.state.synced.val < 18446744073709551616 :=
          U64.lt_succ_max l.state.synced
        have hmp : l.state.acked.val + pending.val = l.state.synced.val := by
          omega
        have hval : (core.num.U64.saturating_add l.state.acked pending).val
            = l.state.synced.val := by
          show (Nat.min ((UScalar.max UScalarTy.U64 : Nat))
              (l.state.acked.val + pending.val)
              % 18446744073709551616) = _
          have hminlt : Nat.min ((UScalar.max UScalarTy.U64 : Nat))
              (l.state.acked.val + pending.val)
              < 18446744073709551616 :=
            Nat.lt_of_le_of_lt (Nat.min_le_left _ _) (by omega)
          rw [Nat.mod_eq_of_lt hminlt]
          rw [hmp]
          exact Nat.min_eq_right (by omega)
        have hsat : core.num.U64.saturating_add l.state.acked pending
            = l.state.synced := UScalar.eq_imp _ _ hval
        have hadd : l.state.acked + pending = ok l.state.synced := by
          have hza := UScalar.add_equiv l.state.acked pending
          cases haddc : l.state.acked + pending with
          | ok a =>
              rw [haddc] at hza
              obtain ⟨_, haval, _⟩ := hza
              exact congrArg ok (UScalar.eq_imp a l.state.synced (by rw [haval, hmp]))
          | fail e =>
              rw [haddc] at hza
              exfalso
              rw [hmp, hbits] at hza
              omega
          | div =>
              rw [haddc] at hza
              cases hza
        simp only [bind_tc_ok]
        unfold wal.wal_state_kernel.wal_ack
        simp only [lift, bind_tc_ok]
        rw [hsat, hadd]
        simp only [bind_tc_ok]
        split
        · rfl
        · next hbad =>
            exact absurd (by simp [UScalar.le_equiv]) hbad
    | fail e =>
        have hzf := UScalar.sub_equiv l.state.synced l.state.acked
        rw [hsub] at hzf
        have hAval : l.state.acked.val ≤ l.state.synced.val := by
          simpa [UScalar.le_equiv] using hle
        omega
    | div =>
        have hzf := UScalar.sub_equiv l.state.synced l.state.acked
        rw [hsub] at hzf
        cases hzf

/-! ## RFC-0215 P1.2 — iff gêmea do átomo `catalog:d1_modelo` sobre
a cópia do d1_modelo neste extrato (perna da coroa composta). -/

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- Twin of `d1m_ok_true` (D1Modelo.lean) over this extract's copy. -/
def wa_d1m_ok_true (s : wal.wal_state_kernel.WalState)
    (rec_end cut : U64) : Prop :=
  ∃ b, wal.wal_state_kernel.inv_wal s = ok b ∧
    (b = false ∨ b = true ∧
      (¬(rec_end <= s.acked) ∨
        rec_end <= s.acked ∧
        ∃ cm, env_crash_kernel.CrashModel.of s.written s.synced = ok cm ∧
          ∃ b1, env_crash_kernel.crash_legal cm cut = ok b1 ∧
            (b1 = false ∨ b1 = true ∧ cut >= rec_end)))

/-- Twin of `d1m_loses` (D1Modelo.lean) over this extract's copy. -/
def wa_d1m_loses (s : wal.wal_state_kernel.WalState)
    (rec_end cut : U64) : Prop :=
  ∃ b, wal.wal_state_kernel.inv_wal s = ok b ∧ b = true ∧
    rec_end <= s.acked ∧
      ∃ cm, env_crash_kernel.CrashModel.of s.written s.synced = ok cm ∧
        ∃ b1, env_crash_kernel.crash_legal cm cut = ok b1 ∧
          b1 = true ∧ cut < rec_end

/-- RFC-0215 P1.2 perna: a iff do átomo `catalog:d1_modelo`
(`d1_modelo_fate_iff`, D1Modelo.lean) reprovada com a mesma técnica
sobre a CÓPIA do `d1_modelo` que vive neste extrato — os dois
ambientes não importam juntos (nomes gerados na raiz colidem), e a
coroa composta (`ComposeProductCrown.lean`) cita ESTA gêmea sobre o
ledger da espinha. Mesma sentença, corpo extraído não reaberto. -/
theorem wa_d1_modelo_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (rec_end cut : U64) (v : Bool),
      (d1_modelo_kernel.d1_modelo s rec_end cut = ok v) ↔
        ((v = true ∧ wa_d1m_ok_true s rec_end cut) ∨
          (v = false ∧ wa_d1m_loses s rec_end cut)) := by
  intro s rec_end cut v
  constructor
  · intro hval
    unfold d1_modelo_kernel.d1_modelo at hval
    obtain ⟨ b, hb, hval ⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hb' =>
        split at hval
        · next hle =>
            obtain ⟨ cm, hcm, hval ⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨ b1, hb1, hval ⟩ := bind_ok_inv _ _ _ hval
            split at hval
            · next hb1' =>
                have hd : decide (cut >= rec_end) = v := Result.ok.inj hval
                cases v with
                | true =>
                    have hge : cut >= rec_end := of_decide_eq_true hd
                    exact Or.inl ⟨rfl, ⟨b, hb, Or.inr ⟨hb',
                      Or.inr ⟨hle, cm, hcm, b1, hb1,
                        Or.inr ⟨hb1', hge⟩⟩⟩⟩⟩
                | false =>
                    have hNG : ¬(cut >= rec_end) := of_decide_eq_false hd
                    have hlt : cut < rec_end := by
                      have hN : ¬(rec_end <= cut) := by
                        simpa [ge_iff_le] using hNG
                      rw [UScalar.le_equiv] at hN
                      rw [UScalar.lt_equiv]
                      omega
                    exact Or.inr ⟨rfl, ⟨b, hb, hb', hle, cm, hcm,
                      b1, hb1, hb1', hlt⟩⟩
            · next hb1n =>
                have hb1F : b1 = false := by
                  simpa [Bool.not_eq_true] using hb1n
                have hv : v = true := (Result.ok.inj hval).symm
                exact Or.inl ⟨hv, ⟨b, hb, Or.inr ⟨hb',
                  Or.inr ⟨hle, cm, hcm, b1, hb1, Or.inl hb1F⟩⟩⟩⟩
        · next hnle =>
            have hv : v = true := (Result.ok.inj hval).symm
            exact Or.inl ⟨hv, ⟨b, hb, Or.inr ⟨hb', Or.inl hnle⟩⟩⟩
    · next hb' =>
        have hbF : b = false := by simpa [Bool.not_eq_true] using hb'
        have hv : v = true := (Result.ok.inj hval).symm
        exact Or.inl ⟨hv, ⟨b, hb, Or.inl hbF⟩⟩
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, b, hb, hbr⟩ := hh
        subst hv
        unfold d1_modelo_kernel.d1_modelo
        rw [hb]
        simp only [Aeneas.Std.bind_tc_ok]
        rcases hbr with hbF | ⟨hbt, hinner⟩
        · rw [hbF, if_neg (by simp)]
        · rw [hbt, if_pos rfl]
          rcases hinner with hnle | ⟨hle, cm, hcm, b1, hb1, hb1r⟩
          · rw [if_neg hnle]
          · rw [if_pos hle]
            rw [hcm]
            simp only [Aeneas.Std.bind_tc_ok]
            rw [hb1]
            simp only [Aeneas.Std.bind_tc_ok]
            rcases hb1r with hb1F | ⟨hb1T, hge⟩
            · rw [hb1F, if_neg (by simp)]
            · rw [hb1T, if_pos rfl]
              rw [decide_eq_true hge]
    | inr hh =>
        obtain ⟨hv, b, hb, hbt, hle, cm, hcm, b1, hb1, hb1T, hlt⟩ := hh
        subst hv
        unfold d1_modelo_kernel.d1_modelo
        rw [hb]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [hbt, if_pos rfl, if_pos hle]
        rw [hcm]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [hb1]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [hb1T, if_pos rfl]
        have hNG : ¬(cut >= rec_end) := by
          have hN : ¬(rec_end <= cut) := by
            rw [UScalar.le_equiv]
            rw [UScalar.lt_equiv] at hlt
            omega
          simpa [ge_iff_le] using hN
        rw [decide_eq_false_iff_not.mpr hNG]
