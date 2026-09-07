-- Theorems over Aeneas extract of world_kernel.rs (RFC-0059 trajectory).
-- Option<&'static str> and HashMap fold holes patched in aeneas_world.sh.
import Aeneas
import WorldKernel
open Aeneas.Std Result
open pedra_aeneas_world_kernel

def sample (term snap applied : U64) : TrajectorySample :=
  {
    step := 0#u32,
    node := 1#u64,
    range := 1#u64,
    term,
    snapshot_index := snap,
    applied_index := applied
  }

/-- Catalog entry: applied watermark regression is reported. -/
theorem trajectory_violation_applied :
    trajectory_violation (sample (1#u64) (4#u64) (7#u64))
      (sample (1#u64) (4#u64) (3#u64)) =
      ok (some (toStr "applied_index")) := by
  unfold trajectory_violation sample
  simp

/-- AS-IS dente: applied regression is blessed. -/
theorem trajectory_violation_as_is_dente :
    trajectory_violation_as_is (sample (1#u64) (4#u64) (7#u64))
      (sample (1#u64) (4#u64) (3#u64)) =
      ok none := by
  unfold trajectory_violation_as_is sample
  simp
