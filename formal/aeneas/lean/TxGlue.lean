-- Theorems over Aeneas extract of tx_glue_kernel.rs
import Aeneas
import TxGlueKernel
open Aeneas.Std Result
open pedra_aeneas_tx_glue_kernel

theorem tx_range_keep_committed :
    tx_range_action true false = ok TxRangeAction.KeepCommitted := by
  unfold tx_range_action
  rfl
