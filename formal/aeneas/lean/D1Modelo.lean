-- Theorems over Aeneas extract of d1_modelo_kernel.rs (RFC-0166 P1.3).
import Aeneas
import D1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_d1_modelo_kernel

/-- Catalog entry: rec_end past acked is vacuously true. -/
theorem d1_modelo_unacked_vacuous :
    d1_modelo_kernel.d1_modelo
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (32#u64) (0#u64) = ok true := by
  unfold d1_modelo_kernel.d1_modelo
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- AS-IS dente: a cut below the barrier is treated as legal and the
    corollary fails (fixed kernel is vacuously true on that cut). -/
theorem d1_modelo_as_is_dente :
    d1_modelo_kernel.d1_modelo_as_is
      { acked := 10#u64, synced := 10#u64, written := 10#u64 }
      (10#u64) (3#u64) = ok false := by
  unfold d1_modelo_kernel.d1_modelo_as_is
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body]

/-- Honest put: append then Honest `wal_sync` then `wal_ack` of the gap. -/
theorem put_ok_append_sync_ack :
    d1_modelo_kernel.put_ok
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (96#u64)
      = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
  unfold d1_modelo_kernel.put_ok
  have happ :
      wal.wal_state_kernel.wal_append
        { acked := 0#u64, synced := 0#u64, written := 0#u64 } (96#u64)
        = ok { acked := 0#u64, synced := 0#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_append
    rfl
  have hsync :
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
  simp [happ, hsync, hsub, hack]

/-- AS-IS dente: Lying sync plus ack-past-barrier (acked > synced). -/
theorem put_ok_as_is_acks_unsynced :
    d1_modelo_kernel.put_ok_as_is
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (96#u64)
      = ok { acked := 96#u64, synced := 0#u64, written := 96#u64 } := by
  unfold d1_modelo_kernel.put_ok_as_is
  unfold wal.wal_state_kernel.wal_append
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]
  rfl

/-! ## RFC-0215 P0.2 — coroa de produto no degrau átomo (modelo ×4) -/

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- D1 modelo mantém: pelas veredas ok do modelo — invariante falso,
registro ainda não ackado, crash ilegal, ou corte cobrindo o registro
(`inv_wal`/`CrashModel.of`/`crash_legal` citados, corpos não reabertos). -/
def d1m_ok_true (s : wal.wal_state_kernel.WalState)
    (rec_end cut : U64) : Prop :=
  ∃ b, wal.wal_state_kernel.inv_wal s = ok b ∧
    (b = false ∨ b = true ∧
      (¬(rec_end <= s.acked) ∨
        rec_end <= s.acked ∧
        ∃ cm, env_crash_kernel.CrashModel.of s.written s.synced = ok cm ∧
          ∃ b1, env_crash_kernel.crash_legal cm cut = ok b1 ∧
            (b1 = false ∨ b1 = true ∧ cut >= rec_end)))

/-- D1 modelo viola: registro ackado, crash legal e corte antes do
fim do registro — ack que o crash desfaz. -/
def d1m_loses (s : wal.wal_state_kernel.WalState)
    (rec_end cut : U64) : Prop :=
  ∃ b, wal.wal_state_kernel.inv_wal s = ok b ∧ b = true ∧
    rec_end <= s.acked ∧
      ∃ cm, env_crash_kernel.CrashModel.of s.written s.synced = ok cm ∧
        ∃ b1, env_crash_kernel.crash_legal cm cut = ok b1 ∧
          b1 = true ∧ cut < rec_end

/-- RFC-0215 P0.2 1/4 (atom `catalog:d1_modelo`, entry `d1_modelo`):
o desfecho da máquina D1 é exatamente a decisão que o spec nomeia —
`ok false` somente na vereda ackado+crash-legal+corte-antes-do-fim
(o buraco do mutante AS-IS `crash_legal_as_is`, que aceita corte
abaixo da barreira); `ok true` pelas demais veredas ok. -/
theorem d1_modelo_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState) (rec_end cut : U64) (v : Bool),
      (d1_modelo_kernel.d1_modelo s rec_end cut = ok v) ↔
        ((v = true ∧ d1m_ok_true s rec_end cut) ∨
          (v = false ∧ d1m_loses s rec_end cut)) := by
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
