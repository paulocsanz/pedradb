-- Theorems over Aeneas extract of rpc_mode_kernel.rs
-- (RFC-0067 P0 DST Queued-RPC pin). Payment is the linked rustc bodies;
-- the former cfg(verus_keep_ghost) stand-in was deleted. No holes here.
import Aeneas
import RpcModeKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_rpc_mode_kernel

/-- DST pin teeth: a pinned Queued world refuses a Direct request. -/
theorem allow_direct_rpc_pin_refuses :
    allow_direct_rpc true true = ok false := by
  unfold allow_direct_rpc
  rfl

/-- DST pin teeth: unpinned still admits Direct. -/
theorem allow_direct_rpc_unpinned_allows_direct :
    allow_direct_rpc false true = ok true := by
  unfold allow_direct_rpc
  rfl

/-- DST pin teeth: a Queued request is always admitted. -/
theorem allow_direct_rpc_queued_always_admitted :
    allow_direct_rpc true false = ok true := by
  unfold allow_direct_rpc
  rfl

/-- AS-IS 0067 dente: the pin does not stick — Direct always admitted. -/
theorem allow_direct_rpc_as_is_dente :
    allow_direct_rpc_as_is true true = ok true := by
  unfold allow_direct_rpc_as_is
  rfl
