-- Theorems over Aeneas extract of lookup_kernel.rs
import Aeneas
import LookupKernel
open Aeneas.Std Result
open pedra_aeneas_lookup_kernel

theorem snap_is_empty_zero :
    snap_is_empty 0#u64 = ok true := by
  unfold snap_is_empty
  rfl

theorem snap_is_empty_as_is_dente :
    snap_is_empty_as_is 0#u64 = ok false := by
  unfold snap_is_empty_as_is
  rfl

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:snap_empty`):
    a snapshot is empty EXACTLY when its sequence is zero — fate
    forall over the extracted body (RFC-0170 P2.4); the AS-IS mutant
    never sees an empty snapshot (the lie the DST plant
    `snap_is_empty_on_live_zero_is_not_ok` refutes). -/
theorem snap_empty_fate_iff :
    ∀ (seq : U64) (v : Bool),
      (snap_is_empty seq = ok v) ↔ v = decide (seq = 0#u64) := by
  intro seq v
  unfold snap_is_empty
  simp
  exact eq_comm

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:snap_below_watermark`):
    a snapshot is below the watermark EXACTLY when its sequence is
    older than the earliest visible — fate forall over the extracted
    body (RFC-0170 P2.4); the AS-IS mutant never drops below (the
    lie the DST plant `snap_below_watermark_on_live_below_is_not_ok`
    refutes). -/
theorem snap_below_watermark_fate_iff :
    ∀ (seq earliest : U64) (v : Bool),
      (snap_below_watermark seq earliest = ok v) ↔
        v = decide (seq < earliest) := by
  intro seq earliest v
  unfold snap_below_watermark
  simp
  exact eq_comm

/-- RFC-0213 P0.2 (storage cadence, atom `catalog:mem_point_decides`):
    the memtable point verdict is the hit flag itself — fate forall
    over the extracted body (RFC-0170 P2.4); the AS-IS mutant always
    reports a miss (the lie the DST plant
    `mem_point_decides_on_live_hit_is_not_ok` refutes). -/
theorem mem_point_decides_fate_iff :
    ∀ (has_point v : Bool),
      (mem_point_decides has_point = ok v) ↔ v = has_point := by
  intro has_point v
  unfold mem_point_decides
  cases has_point <;> cases v <;> simp
