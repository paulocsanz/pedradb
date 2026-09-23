-- Theorems over Aeneas extract of scan_readahead_kernel.rs (RFC-0222 P0.7).
import Aeneas
import ScanReadaheadKernel
open Aeneas.Std Result
open pedra_aeneas_scan_readahead_kernel

/-- RFC-0222 P0.7 (atom `catalog:scan_readahead_window`): a fitting/hot
store never issues WILLNEED — the Fire-118/v69 lesson on the read side.
Fate forall over the extracted body. -/
theorem scan_readahead_window_hot_never :
    ∀ (blocks : Slice (U64 × U64)) (at1 : Usize),
      scan_readahead_window blocks at1 false =
        ok ScanReadaheadWindow.NONE := by
  intro blocks at1
  unfold scan_readahead_window
  rfl

/-- AS-IS dente: today's engine never reads ahead. -/
theorem scan_readahead_window_as_is_always_none :
    ∀ (blocks : Slice (U64 × U64)) (at1 : Usize) (bounded : Bool),
      scan_readahead_window_as_is blocks at1 bounded =
        ok ScanReadaheadWindow.NONE := by
  intro blocks at1 bounded
  unfold scan_readahead_window_as_is
  rfl

