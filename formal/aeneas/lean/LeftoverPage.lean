-- Theorems over Aeneas extract of leftover_page_kernel.rs (RFC-0222 P0.7).
import Aeneas
import LeftoverPageKernel
open Aeneas.Std Result
open pedra_aeneas_leftover_page_kernel

/-- RFC-0222 P0.7 (atom `catalog:leftover_page_advice`): drop leftover
SST pages iff budget=0 AND no covering live family AND store above the
warm cap. Fate forall over the extracted body. AS-IS always KeepDefault
(Fire-118: today's engine never advises leftover pages). -/
theorem leftover_page_advice_fate_iff :
    ∀ (budget bytes cap : U64) (covered : Bool),
      leftover_page_advice budget bytes cap covered =
        (if budget != 0#u64 then ok LeftoverPageAdvice.KeepDefault
         else if covered then ok LeftoverPageAdvice.KeepCovered
         else if bytes <= cap then ok LeftoverPageAdvice.KeepHot
         else ok LeftoverPageAdvice.Drop) := by
  intro budget bytes cap covered
  unfold leftover_page_advice
  rfl

/-- AS-IS tooth: never drop leftover pages. -/
theorem leftover_page_advice_as_is_always_keep :
    ∀ (budget bytes cap : U64) (covered : Bool),
      leftover_page_advice_as_is budget bytes cap covered =
        ok LeftoverPageAdvice.KeepDefault := by
  intro budget bytes cap covered
  unfold leftover_page_advice_as_is
  rfl
