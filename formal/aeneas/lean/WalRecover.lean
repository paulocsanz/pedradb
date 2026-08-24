-- Theorems over the Aeneas extract of production wal/recover_kernel.rs
-- (RFC-0053 P1.3 / RFC-0056 P1.2): second machine (not the Verus twin).
import Aeneas
import WalRecoverKernel
open Aeneas Std Result
open pedra_aeneas_wal_recover_kernel

/-- A complete record is always kept — the collector never drops a
record silently. -/
theorem record_is_kept (prefix_n : Std.U64) (can_skip : Bool)
    (skips : Std.U64) (in_resync : Bool) :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Record
      prefix_n can_skip skips in_resync =
      ok recover_kernel.RecoverAct.KeepRecord := by
  rfl

/-- G8: CRC at a fresh alignment fail-stops (a real record with a bad
checksum is never skipped). -/
theorem crc_fresh_alignment_fail_stops (prefix_n : Std.U64) (can_skip : Bool)
    (skips : Std.U64) :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Crc
      prefix_n can_skip skips false =
      ok recover_kernel.RecoverAct.FailStop := by
  cases can_skip <;> rfl

/-- F4: torn first record on an empty prefix fail-stops (not a silent
empty WAL). -/
theorem empty_prefix_torn_fail_stops :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
      (0#u64) false (0#u64) false =
      ok recover_kernel.RecoverAct.FailStop := by
  rfl

/-- A torn tail over a live prefix keeps the prefix (the discard is the
prefix boundary, not the whole log). -/
theorem prefix_torn_keeps :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
      (1#u64) false (0#u64) false =
      ok recover_kernel.RecoverAct.KeepPrefix := by
  rfl

/-- F14: an orphan fragment fail-stops (never clean EOF). -/
theorem orphan_fragment_fail_stops :
    recover_kernel.recover_collect_act recover_kernel.RecoverKind.OrphanFragment
      (1#u64) true (0#u64) false =
      ok recover_kernel.RecoverAct.FailStop := by
  rfl

/-- AS-IS teeth: torn empty prefix becomes a silent Stop (empty WAL)
where the fixed kernel fail-stops — the F4 silent-wrong. -/
theorem as_is_torn_is_silent_eof :
    recover_kernel.recover_collect_act_as_is recover_kernel.RecoverKind.Truncated
      (0#u64) false (0#u64) =
      ok recover_kernel.RecoverAct.Stop ∧
      recover_kernel.recover_collect_act recover_kernel.RecoverKind.Truncated
        (0#u64) false (0#u64) false =
        ok recover_kernel.RecoverAct.FailStop := by
  constructor <;> rfl

/-- AS-IS teeth: CRC at a fresh alignment silently resyncs where the
fixed kernel fail-stops — the G8 silent-wrong. -/
theorem as_is_crc_resyncs :
    recover_kernel.recover_collect_act_as_is recover_kernel.RecoverKind.Crc
      (3#u64) true (0#u64) =
      ok recover_kernel.RecoverAct.Resync ∧
      recover_kernel.recover_collect_act recover_kernel.RecoverKind.Crc
        (3#u64) true (0#u64) false =
        ok recover_kernel.RecoverAct.FailStop := by
  constructor <;> rfl

/-- AS-IS teeth: `is_length_resyncable_as_is` misclassifies CRC as
length-resyncable — the misclassification the fixed classifier refuses. -/
theorem as_is_crc_not_length_resyncable :
    recover_kernel.is_length_resyncable recover_kernel.RecoverKind.Crc = ok false ∧
      recover_kernel.is_length_resyncable_as_is recover_kernel.RecoverKind.Crc = ok true := by
  constructor <;> rfl

