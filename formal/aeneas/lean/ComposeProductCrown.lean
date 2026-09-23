-- Cross-lib composition (RFC-0215 P1.2): the product crown —
-- espinha→coroa. For every RFC-0214 `spine_reach` path, the recovered
-- ledger passes BOTH legs of the product promise:
--   * the model leg `d1_modelo = ok true` — cited through the iff
--     twin of the `catalog:d1_modelo` atom (`wa_d1_modelo_fate_iff`,
--     WriteAck.lean — the same sentence over this environment's copy
--     of the body; the extracted body is NOT reopened here). Only the
--     callees (`inv_wal`, `CrashModel.of`, `crash_legal`) open once,
--     as the composition's assembly work;
--   * the spec leg `d1_holds = ok true` for every positional view of
--     the acked prefix — cited through the P0.1 atom
--     `d1_holds_fate_iff` (`Properties.lean`).
-- Registration rule: a row needs a single catalog pair/entry; this
-- composition spans the spine atoms + `d1_modelo` + `d1_holds` — no
-- row (reason dated in findings, same as the other compose libs).
import Aeneas
import ComposeDurabilitySpine
import Properties
open Aeneas.Std Result
open pedra_aeneas_write_ack_kernel
open pedra_aeneas_properties_kernel

/-- Assembly: `CrashModel.of` lands ok on some model (the witness is
inferred per `Ord.min` branch — the min body is not case-analyzed). -/
private theorem crash_model_of_ok (written synced : U64) :
    ∃ cm, env_crash_kernel.CrashModel.of written synced = ok cm := by
  unfold env_crash_kernel.CrashModel.of
  simp only [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt, liftFun2,
    Aeneas.Std.bind_tc_ok]
  split
  · next _ => exact ⟨_, rfl⟩
  · next _ => exact ⟨_, rfl⟩

/-- Assembly: `crash_legal` lands ok on a branch-form `b1` — for any
    cut past `rec_end`, every outcome fits the twin's disjunct
    (illegal crash gives `false`; legal crash gives the cut verdict). -/
private theorem crash_legal_ok_branches
    (m : env_crash_kernel.CrashModel) (cut rec_end : U64)
    (hge : rec_end <= cut) :
    ∃ b1, env_crash_kernel.crash_legal m cut = ok b1 ∧
      (b1 = false ∨ b1 = true ∧ cut >= rec_end) := by
  by_cases hsync : m.synced <= cut
  · by_cases hw : cut <= m.written
    · refine ⟨true, ?_, Or.inr ⟨rfl, hge⟩⟩
      unfold env_crash_kernel.crash_legal
      rw [if_pos hsync, decide_eq_true hw]
    · refine ⟨false, ?_, Or.inl rfl⟩
      unfold env_crash_kernel.crash_legal
      rw [if_pos hsync, decide_eq_false_iff_not.mpr hw]
  · refine ⟨false, ?_, Or.inl rfl⟩
    unfold env_crash_kernel.crash_legal
    rw [if_neg hsync]

/-- RFC-0215 P1.2: the product crown — every spine-reachable ledger
    passes the model leg AND the spec leg: the acked prefix is durable
    in the machine (`d1_modelo`, via the iff twin — body not reopened)
    and in the positional predictor (`d1_holds`, via the P0.1 atom),
    for every torn cut and every positional view of the prefix. -/
theorem product_crown_every_reach :
    ∀ (k : Nat) (l : write_ack_kernel.WriteAckLedger)
      (cut : U64) (flags : Slice Bool) (survives : Usize),
      spine_reach k l →
        l.state.acked <= cut →
        (∀ i : Nat, (hi : i < flags.val.length) →
          flags.val[i] = true → i < l.state.acked.val) →
        l.state.acked.val <= survives.val →
        d1_modelo_kernel.d1_modelo l.state l.state.acked cut = ok true ∧
          d1_holds flags survives = ok true := by
  intro k l cut flags survives hr hcut hflags hsurv
  obtain ⟨h1, h2⟩ := spine_inv_every_reach k l hr
  have hIW : wal.wal_state_kernel.inv_wal l.state = ok true := by
    unfold wal.wal_state_kernel.inv_wal
    split
    · exact congrArg ok (decide_eq_true h2)
    · next hbad => exact absurd h1 hbad
  obtain ⟨cm, hcm⟩ := crash_model_of_ok l.state.written l.state.synced
  obtain ⟨b1, hb1, hb1r⟩ := crash_legal_ok_branches cm cut l.state.acked hcut
  refine ⟨?_, ?_⟩
  · rw [wa_d1_modelo_fate_iff]
    exact Or.inl ⟨rfl, ⟨true, hIW, Or.inr ⟨rfl, Or.inr ⟨le_refl _, cm, hcm,
      b1, hb1, hb1r⟩⟩⟩⟩
  · refine (d1_holds_fate_iff flags survives true).mpr (Or.inl ⟨rfl, ?_⟩)
    intro i hi htrue
    have hik : i < l.state.acked.val := hflags i hi htrue
    omega
