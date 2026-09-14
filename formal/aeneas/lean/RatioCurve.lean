-- Theorems over Aeneas extract of ratio_curve_kernel.rs (RFC-0222 P0.7).
-- Whole-file extract is blocked by `&'static str` (GetSideAnchor);
-- --start-from isolates `cold_permille`, the integer predicate the curve
-- is built on.
import Aeneas
import RatioCurveKernel
open Aeneas.Std Result
open pedra_aeneas_ratio_curve_kernel.ratio_curve_kernel

/-- RFC-0222 P0.7 (atom `catalog:cold_permille`): cold fraction is 0 when
the store fits the warm cap (or is empty), else min(1000,
(store-warm)*1000/store). Fate forall over the extracted body. -/
theorem cold_permille_fate_iff :
    ∀ (store_bytes warm_cap : U64),
      cold_permille store_bytes warm_cap =
        (if store_bytes <= warm_cap then ok 0#u64
         else if store_bytes = 0#u64 then ok 0#u64
         else do
           let i ← store_bytes - warm_cap
           let i1 ← i * 1000#u64
           let i2 ← i1 / store_bytes
           core.cmp.Ord.min.trait_default core.cmp.OrdU64 i2 1000#u64) := by
  intro store_bytes warm_cap
  unfold cold_permille
  rfl
