-- Theorems over Aeneas extract of t1_modelo_kernel.rs (RFC-0166 P2.2).
import Aeneas
import T1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_t1_modelo_kernel

/-- Catalog entry: empty TX recovers to T1. -/
theorem t1_modelo_empty :
    t1_modelo_kernel.t1_modelo
      { staged := 0#u64, visible := 0#u64, committed := false,
        aborted := false, fenced := false } = ok true := by
  unfold t1_modelo_kernel.t1_modelo
  unfold t1_modelo_kernel.tx_recover
  unfold txn_kernel.leftover_fate
  unfold t1_modelo_kernel.t1_holds_of
  rfl

/-- AS-IS dente: mid-apply partial visibility is not recovered. -/
theorem t1_modelo_as_is_dente :
    t1_modelo_kernel.t1_modelo_as_is
      { staged := 2#u64, visible := 1#u64, committed := false,
        aborted := false, fenced := false } = ok false := by
  unfold t1_modelo_kernel.t1_modelo_as_is
  unfold t1_modelo_kernel.tx_recover_as_is
  unfold txn_kernel.leftover_txn_is_aborted_as_is
  unfold txn_kernel.leftover_fate_as_is
  unfold t1_modelo_kernel.t1_holds_of
  rfl
