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
