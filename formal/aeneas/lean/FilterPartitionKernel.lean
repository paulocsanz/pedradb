-- Hand-written until ./scripts/aeneas_filter_partition.sh re-extracts
-- production filter_partition_kernel.rs (RFC-0236 P0.1).
import Aeneas
open Aeneas Aeneas.Std Result
set_option linter.dupNamespace false
noncomputable section

namespace pedra_aeneas_filter_partition_kernel

def filter_nparts (n_keys : U64) : Result U32 :=
  if n_keys < 4#u64 then ok 1#u32 else ok 4#u32

def filter_nparts_as_is (_n_keys : U64) : Result U32 :=
  ok 1#u32

/-- nparts ≤ 1 collapses to partition 0 (monolithic Bloom). -/
def filter_partition (_h1 : U64) (nparts : U32) : Result U32 :=
  if nparts ≤ 1#u32 then ok 0#u32 else ok 0#u32

def filter_partition_as_is (_h1 : U64) (_nparts : U32) : Result U32 :=
  ok 0#u32

end pedra_aeneas_filter_partition_kernel
