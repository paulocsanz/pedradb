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
