-- Theorems over Aeneas extract of rpc_mode_kernel.rs
import Aeneas
import RpcModeKernel
open Aeneas.Std Result
open pedra_aeneas_rpc_mode_kernel

theorem allow_direct_rpc_pin_refuses :
    allow_direct_rpc true true = ok false := by
  unfold allow_direct_rpc
  rfl

theorem allow_direct_rpc_as_is_dente :
    allow_direct_rpc_as_is true true = ok true := by
  unfold allow_direct_rpc_as_is
  rfl
