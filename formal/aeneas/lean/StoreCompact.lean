-- Theorems over Aeneas extract of store compact_kernel.rs
-- (RFC-0002 P13 / F27 / F28 / RFC-0100 / RFC-0109). Payment is the linked
-- rustc bodies; the former cfg(verus_keep_ghost) stand-in was deleted.
-- Fail-closed: this file must not contain a hole.
import Aeneas
import StoreCompactKernel
open Aeneas.Std Result
open pedra_aeneas_store_compact_kernel

/-- F28: an offline peer still counts in the compact watermark. -/
theorem peer_counts_offline_still_counts :
    peer_counts_for_compact false = ok true := by
  rfl

/-- F28: a participating peer counts too. -/
theorem peer_counts_participating_counts :
    peer_counts_for_compact true = ok true := by
  rfl

/-- AS-IS F28 dente: only live peers — compact past offline applied. -/
theorem peer_counts_as_is_skips_offline :
    peer_counts_for_compact_as_is false = ok false := by
  rfl

/-- F27: nothing durable at applied 0 — no compact. -/
theorem compact_ready_zero_not_ready :
    compact_ready (0#u64) = ok false := by
  unfold compact_ready
  rfl

/-- F27: durable entries make compact ready. -/
theorem compact_ready_positive_ready :
    compact_ready (3#u64) = ok true := by
  unfold compact_ready
  rfl

/-- F27: the entry must be in the log (through 0 refused). -/
theorem may_compact_through_zero_false :
    may_compact_through 0#u64 0#u64 1#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- F27: missing term at through (entry absent) refuses. -/
theorem may_compact_through_missing_term_refuses :
    may_compact_through 0#u64 5#u64 0#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- F27: a present entry with a term may compact. -/
theorem may_compact_through_present_term_allows :
    may_compact_through 0#u64 5#u64 2#u64 = ok true := by
  unfold may_compact_through
  rfl

/-- F27: already covered by the snapshot — refuse. -/
theorem may_compact_through_snapshot_covered_refuses :
    may_compact_through 5#u64 5#u64 2#u64 = ok false := by
  unfold may_compact_through
  rfl

/-- AS-IS F27 dente: compacts even when the entry is missing. -/
theorem may_compact_through_as_is_missing_term_allows :
    may_compact_through_as_is 0#u64 5#u64 0#u64 = ok true := by
  unfold may_compact_through_as_is
  rfl

/-- Floor after compact through n is n+1. -/
theorem compact_index_floor_advances :
    compact_index_floor (7#u64) = ok 8#u64 := by
  unfold compact_index_floor
  have h : core.num.U64.saturating_add 7#u64 1#u64 = 8#u64 := by native_decide
  simp [h]

/-- u64::MAX saturates — floor never wraps to 0. -/
theorem compact_index_floor_max_saturates :
    compact_index_floor (18446744073709551615#u64)
      = ok 18446744073709551615#u64 := by
  unfold compact_index_floor
  have h : core.num.U64.saturating_add 18446744073709551615#u64 1#u64
      = 18446744073709551615#u64 := by native_decide
  simp [h]

/-- RFC-0100/0109: compact stops before an un-left joint. -/
theorem compact_through_unleft_caps_below_joint :
    compact_through_unleft (5#u64) (some 3#u64) = ok 2#u64 := by
  unfold compact_through_unleft
  have hgt : (3#u64 > 0#u64) = true := by native_decide
  have hle : (3#u64 <= 5#u64) = true := by native_decide
  have hsub : core.num.U64.saturating_sub 3#u64 1#u64 = 2#u64 := by native_decide
  simp [hgt, hle, hsub]

/-- Joint at the exact through index also caps (through j-1). -/
theorem compact_through_unleft_joint_at_through_caps :
    compact_through_unleft (5#u64) (some 5#u64) = ok 4#u64 := by
  unfold compact_through_unleft
  have hgt : (5#u64 > 0#u64) = true := by native_decide
  have hsub : core.num.U64.saturating_sub 5#u64 1#u64 = 4#u64 := by native_decide
  simp [hgt, hsub]

/-- A joint past through does not cap (not applied yet). -/
theorem compact_through_unleft_joint_past_through_no_cap :
    compact_through_unleft (5#u64) (some 6#u64) = ok 5#u64 := by
  unfold compact_through_unleft
  have hgt : (6#u64 > 0#u64) = true := by native_decide
  have hle : (6#u64 <= 5#u64) = false := by native_decide
  simp [hle]

/-- No joint: through unchanged. -/
theorem compact_through_unleft_none_no_cap :
    compact_through_unleft (5#u64) none = ok 5#u64 := by
  rfl

/-- A zero joint is no joint (membership equal already). -/
theorem compact_through_unleft_zero_joint_no_cap :
    compact_through_unleft (5#u64) (some 0#u64) = ok 5#u64 := by
  unfold compact_through_unleft
  simp

/-- AS-IS dente: compact past the un-left joint (the 0096/0100 hole). -/
theorem compact_through_unleft_as_is_past_joint :
    compact_through_unleft_as_is (5#u64) (some 3#u64) = ok 5#u64 := by
  rfl
