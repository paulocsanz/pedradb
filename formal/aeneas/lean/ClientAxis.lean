-- Theorems over Aeneas extract of client_axis_kernel.rs (RFC-0222 P0.7).
import Aeneas
import ClientAxisKernel
open Aeneas.Std Result
open pedra_aeneas_client_axis_kernel

/-- RFC-0222 P0.7 (atom `catalog:pipeline_drain_cap`): the leader drain
cap is `clamp(queued, 1, 256)` — full drain up to the misuse floor,
never empty. Fate forall over the extracted body. AS-IS clamps at 8
(the cap-8 convoy RFC-0201 removed). -/
theorem pipeline_drain_cap_fate_iff :
    ∀ (q : Usize),
      pipeline_drain_cap q =
        core.cmp.impls.OrdUsize.clamp q 1#usize PIPELINE_DRAIN_MAX_MEMBERS := by
  intro q
  unfold pipeline_drain_cap
  rfl

/-- AS-IS tooth: the pre-0201 cap-8 convoy. -/
theorem pipeline_drain_cap_as_is_caps_at_eight :
    ∀ (q : Usize),
      pipeline_drain_cap_as_is q =
        core.cmp.impls.OrdUsize.clamp q 1#usize PIPELINE_DRAIN_CAP_AS_IS := by
  intro q
  unfold pipeline_drain_cap_as_is
  rfl

/-- RFC-0222 P0.7 (atom `catalog:async_merge_policy`): merge iff the env
pin says so, else iff writers outnumber CPUs (ncpu=0 never merges).
Fate forall over the extracted body. AS-IS is env-pin only. -/
theorem async_merge_policy_fate_iff :
    ∀ (writers ncpu : Usize) (forced : Option Bool),
      async_merge_policy writers ncpu forced =
        (match forced with
         | none => if ncpu > 0#usize then ok (writers > ncpu) else ok false
         | some pin => ok pin) := by
  intro writers ncpu forced
  unfold async_merge_policy
  rfl
