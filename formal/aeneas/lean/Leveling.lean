-- Theorems over Aeneas extract of leveling.rs (leveled compaction).
-- RUSTFLAGS=--cfg test so as_is mutants are visible; pick Iterator holes
-- patched to index loops in aeneas_leveling.sh.
import Aeneas
import LevelingKernel
open Aeneas.Std Result
open pedra_aeneas_leveling_kernel

/-- Catalog entry: L0 has no size target. -/
theorem level_target_bytes_l0 (t) :
    level_target_bytes (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes
  rfl

/-- AS-IS dente: L0 target is still zero. -/
theorem level_target_bytes_as_is_l0 (t) :
    level_target_bytes_as_is (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes_as_is
  rfl
