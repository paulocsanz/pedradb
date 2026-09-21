-- Hand-written until ./scripts/aeneas_workload_class.sh re-extracts
-- production workload_class_kernel.rs (RFC-0235 P0.1).
import Aeneas
open Aeneas Aeneas.Std Result
set_option linter.dupNamespace false
noncomputable section

namespace pedra_aeneas_workload_class_kernel

inductive WorkloadClass where
  | PointMiss
  | PointHit
  | ShortRange
  | Sequential
  | WriteBurst
  | Mixed
  deriving Repr, BEq

def workload_class (z0 z1 q w : U64) : Result WorkloadClass :=
  if q > z0 && q > z1 && q > w then ok .ShortRange
  else if w > z0 && w > z1 && w > q then ok .WriteBurst
  else if z0 > z1 && z0 > q && z0 > w then ok .PointMiss
  else if z1 > z0 && z1 > q && z1 > w then ok .PointHit
  else ok .Mixed

def workload_class_as_is (_z0 _z1 _q _w : U64) : Result WorkloadClass :=
  ok .Mixed

end pedra_aeneas_workload_class_kernel
