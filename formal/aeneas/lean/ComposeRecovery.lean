-- RFC-0222 P2.2 — recovery spine: wal_recover × manifest × reopen × vlog
-- COMPOSED. Boot serves (manifest not RefuseOpen, reopen ServeAll, vlog
-- not RefuseOpen) EXACTLY on the conjunction of the three registered
-- refuse/serve atoms. Dual-unfold: the extracted bodies are never opened.
import Aeneas
import Manifest
import Reopen
import VlogGc
import WalRecover
open Aeneas.Std Result
open pedra_aeneas_manifest_kernel
open pedra_aeneas_reopen_kernel
open pedra_aeneas_vlog_gc_kernel
open pedra_aeneas_wal_recover_kernel

/-- RFC-0222 P2.2: a clean boot (manifest not refuse, reopen ServeAll,
    vlog not refuse) holds EXACTLY when the manifest is not corrupt and
    not an inventory with a missing listed SST, reopen carries no damage,
    and the vlog swing is not "use_new with nothing on disk". Dual-unfold
    of `sst_recover_action_refuse_iff_corrupt_or_inventory_missing` ×
    `reopen_outcome_serve_all_iff_damage_none` ×
    `vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk`. -/
theorem recovery_spine_boot_ok_iff :
    ∀ (obs : ManifestObs) (listed : ListedSst)
      (damage : ReopenDamage) (pit esc : Bool)
      (blob wants prim use_new new_ex : Bool),
      (¬ (sst_recover_action obs listed = ok SstRecoverAction.RefuseOpen) ∧
          reopen_outcome damage pit esc = ok ReopenOutcome.ServeAll ∧
          ¬ (vlog_recover_action blob wants prim use_new new_ex
              = ok VlogRecoverAction.RefuseOpen)) ↔
        (¬ (obs = ManifestObs.Corrupt ∨
              (obs = ManifestObs.Inventory ∧
                ∃ i, listed = ListedSst.Missing i)) ∧
          damage = ReopenDamage.None ∧
          ¬ (blob = false ∧ wants = true ∧ prim = false ∧ use_new = true ∧
              new_ex = false)) := by
  intro obs listed damage pit esc blob wants prim use_new new_ex
  constructor
  · intro ⟨hs, hr, hv⟩
    refine ⟨?_, ?_, ?_⟩
    · intro h
      exact hs ((sst_recover_action_refuse_iff_corrupt_or_inventory_missing
        obs listed).mpr h)
    · exact (reopen_outcome_serve_all_iff_damage_none damage pit esc).mp hr
    · intro h
      exact hv
        ((vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk
          blob wants prim use_new new_ex).mpr h)
  · intro ⟨hs, hr, hv⟩
    refine ⟨?_, ?_, ?_⟩
    · intro h
      exact hs ((sst_recover_action_refuse_iff_corrupt_or_inventory_missing
        obs listed).mp h)
    · exact (reopen_outcome_serve_all_iff_damage_none damage pit esc).mpr hr
    · intro h
      exact hv
        ((vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk
          blob wants prim use_new new_ex).mp h)

/-- WAL collector: a complete record is KeepRecord on every prefix —
    the fourth recovery atom (`catalog:wal_recover` / `recover_collect_act`). -/
theorem recovery_spine_wal_record_kept :
    ∀ (prefix_n : U64) (can_skip : Bool) (skips : U64) (in_resync : Bool),
      recover_kernel.recover_collect_act recover_kernel.RecoverKind.Record
        prefix_n can_skip skips in_resync =
        ok recover_kernel.RecoverAct.KeepRecord :=
  record_is_kept

/-- RFC-0224 P1.1: WAL fragment spine COMPOSTA — um Full on-wire é
    FragKind.Full, Yield no collector, Record nunca é length-resyncable,
    e o guarda físico Continua quando o payload cabe. Dual-unfold de
    `from_record_type_fate_iff` × `fragment_act_fate_iff` ×
    `is_length_resyncable_fate_iff` × `physical_payload_act_fate_iff`.
    Corpos extraídos não abrem. -/
theorem recovery_wal_full_record_spine :
    ∀ (scratch_empty : Bool)
      (length max_payload payload_end block_end block_size : U64)
      (f : recover_kernel.FragKind) (act : recover_kernel.FragAct)
      (resync : Bool) (phys : recover_kernel.PhysicalAct),
      (recover_kernel.FragKind.from_record_type format.RecordType.Full = ok f ∧
          recover_kernel.fragment_act f scratch_empty = ok act ∧
          recover_kernel.is_length_resyncable recover_kernel.RecoverKind.Record
            = ok resync ∧
          ¬(length > max_payload) ∧ ¬(payload_end > block_end) ∧
          recover_kernel.physical_payload_act length max_payload payload_end
            block_end block_size = ok phys) →
        (f = recover_kernel.FragKind.Full ∧
          act = recover_kernel.FragAct.Yield ∧
          resync = false ∧
          phys = recover_kernel.PhysicalAct.Continue) := by
  intro scratch_empty length max_payload payload_end block_end block_size
    f act resync phys ⟨hf, hact, hre, hlen, hend, hphys⟩
  have hf' := (from_record_type_fate_iff format.RecordType.Full f).mp hf
  have hf_full : f = recover_kernel.FragKind.Full := by
    rcases hf' with h0 | hfull | h1 | h2 | h3
    · cases h0.1
    · exact hfull.2
    · cases h1.1
    · cases h2.1
    · cases h3.1
  have hact' := (fragment_act_fate_iff f scratch_empty act).mp hact
  have hyield : act = recover_kernel.FragAct.Yield := by
    rcases hact' with hy | hst | hm1 | hm2 | hl1 | hl2 | hz
    · exact hy.2
    · exact absurd hst.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
    · exact absurd hm1.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
    · exact absurd hm2.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
    · exact absurd hl1.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
    · exact absurd hl2.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
    · exact absurd hz.1 (hf_full ▸ fun h => recover_kernel.FragKind.noConfusion h)
  have hre' := (is_length_resyncable_fate_iff
    recover_kernel.RecoverKind.Record resync).mp hre
  have hfalse : resync = false := by
    rcases hre' with ht | hl | hu | hr | hc | ho | hcrc | hz | hoth
    · cases ht.1
    · cases hl.1
    · cases hu.1
    · exact hr.2
    · cases hc.1
    · cases ho.1
    · cases hcrc.1
    · cases hz.1
    · cases hoth.1
  have hphys' := (physical_payload_act_fate_iff length max_payload payload_end
    block_end block_size phys).mp hphys
  have hcont : phys = recover_kernel.PhysicalAct.Continue := by
    rcases hphys' with hfail | hfail2 | htrunc | hcont
    · exact absurd hfail.1 hlen
    · exact absurd hfail2.2.1 hend
    · exact absurd htrunc.2.1 hend
    · exact hcont.2.2
  exact ⟨hf_full, hyield, hfalse, hcont⟩

/-- RFC-0224 P1.1: first-install Failed recusa abrir — dual-unfold de
    `first_install_action_fate_iff`. -/
theorem recovery_first_install_failed_refuses :
    ∀ (act : FirstInstallAction),
      first_install_action FirstInstallOutcome.Failed = ok act →
        act = FirstInstallAction.RefuseOpen := by
  intro act h
  have := (first_install_action_fate_iff FirstInstallOutcome.Failed act).mp h
  rcases this with h1 | h2 | h3
  · cases h1.1
  · cases h2.1
  · exact h3.2

/-- RFC-0224 P1.1: bulk MANIFEST persist-now EXATAMENTE com sync —
    dual-unfold de `bulk_manifest_persist_fate_fate_iff`. -/
theorem recovery_bulk_manifest_sync_persists_now :
    ∀ (sync : Bool) (fate : BulkManifestFate),
      (bulk_manifest_persist_fate sync = ok fate) ↔
        ((sync = true ∧ fate = BulkManifestFate.PersistNow) ∨
          (sync = false ∧ fate = BulkManifestFate.AmortizeDebt)) :=
  bulk_manifest_persist_fate_fate_iff

/-- RFC-0224 P1.1: blob GC reescreve EXATAMENTE gen inativo com bytes —
    dual-unfold de `blob_gc_action_rewrite_iff_inactive_with_bytes`. -/
theorem recovery_blob_gc_rewrite_iff :
    ∀ (is_active : Bool) (bytes : U64),
      (blob_gc_action is_active bytes = ok BlobGcAction.Rewrite)
        ↔ (is_active = false ∧ bytes > 0#u64) :=
  blob_gc_action_rewrite_iff_inactive_with_bytes

/-- RFC-0224 P1.1: first-install Failed recusa E blob GC do gen ativo
    nunca reescreve — os dois átomos compostos. -/
theorem recovery_failed_install_and_active_gen_never_rewrite :
    ∀ (act : FirstInstallAction) (bytes : U64),
      first_install_action FirstInstallOutcome.Failed = ok act →
        ¬ (blob_gc_action true bytes = ok BlobGcAction.Rewrite) := by
  intro act bytes hinst hgc
  have := recovery_first_install_failed_refuses act hinst
  have ⟨hactive, _⟩ := (recovery_blob_gc_rewrite_iff true bytes).mp hgc
  cases hactive
