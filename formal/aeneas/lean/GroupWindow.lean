-- Theorems over Aeneas extract of group_window_kernel.rs (RFC-0222 P0.7).
import Aeneas
import GroupWindowKernel
open Aeneas.Std Result
open pedra_aeneas_group_window_kernel

/-- RFC-0222 P0.7 (atom `catalog:merge_eligible`): with the window on,
async merge is eligible from 2 writers up, or from a recent peer
(gap-ghost). Fate forall over the extracted body. AS-IS is always
false (RFC-0044 bypass, avg_grp == 1.00). -/
theorem merge_eligible_fate_iff :
    ∀ (writers : Usize) (window_us : U64) (peers_recent : Bool),
      merge_eligible writers window_us peers_recent =
        (if window_us > 0#u64 then
           if writers >= 2#usize then ok true else ok peers_recent
         else ok false) := by
  intro writers window_us peers_recent
  unfold merge_eligible
  rfl

/-- AS-IS dente: no window, no low-writer merge. -/
theorem merge_eligible_as_is_never :
    ∀ (writers : Usize) (window_us : U64),
      merge_eligible_as_is writers window_us = ok false := by
  intro writers window_us
  unfold merge_eligible_as_is
  rfl

/-- RFC-0222 P0.7 (atom `catalog:flight_capped_window_us`): when
`cap_to_flight`, the collect window is min(window, flight-or-seed)
and collapses to 0 below one quiescence slice; otherwise the flat
window. Fate forall over the extracted body. -/
theorem flight_capped_window_us_fate_iff :
    ∀ (window_us flight_ema_us : U64) (cap_to_flight : Bool),
      flight_capped_window_us window_us flight_ema_us cap_to_flight =
        (if cap_to_flight then
           if window_us = 0#u64 then ok window_us
           else do
             let flight ←
               if flight_ema_us = 0#u64
               then ok GROUP_FLIGHT_SEED_US
               else ok flight_ema_us
             let capped ←
               core.cmp.Ord.min.trait_default core.cmp.OrdU64 window_us flight
             if capped < COLLECT_QUIESCE_US then ok 0#u64 else ok capped
         else ok window_us) := by
  intro window_us flight_ema_us cap_to_flight
  unfold flight_capped_window_us
  rfl

/-- A2b (atom `catalog:herd_full`): the mc4 frame is full exactly at
    `HERD_TARGET`. AS-IS never full. -/
theorem herd_full_fate_iff :
    ∀ (batch_len : Usize),
      herd_full batch_len = ok (decide (batch_len ≥ HERD_TARGET)) := by
  intro batch_len
  unfold herd_full
  simp

/-- A2b (atom `catalog:herd_collect_us`): wait `HERD_COLLECT_US` iff the
    frame is not `herd_full` and (active > batch or a recent peer).
    Unfolds `herd_full`. AS-IS never waits. -/
theorem herd_collect_us_fate_iff :
    ∀ (active batch_len : Usize) (peers_recent : Bool),
      herd_collect_us active batch_len peers_recent =
        (do
          let b ← herd_full batch_len
          if b then ok 0#u64
          else if active > batch_len then ok HERD_COLLECT_US
          else if peers_recent then ok HERD_COLLECT_US
          else ok 0#u64) := by
  intro active batch_len peers_recent
  unfold herd_collect_us
  rfl

/-- A2b (atom `catalog:post_group_grace_us`): after a multi-member
    publish, grace-spin iff prev ≥ 2 and the new frame is not `herd_full`.
    Unfolds `herd_full`. AS-IS never spins. -/
theorem post_group_grace_us_fate_iff :
    ∀ (prev_len batch_len : Usize),
      post_group_grace_us prev_len batch_len =
        (if prev_len ≥ 2#usize then
           do
             let b ← herd_full batch_len
             if b then ok 0#u64 else ok HERD_COLLECT_US
         else ok 0#u64) := by
  intro prev_len batch_len
  unfold post_group_grace_us
  rfl
