-- Theorems over Aeneas extract of si_kernel.rs
import Aeneas
import SiKernel
open Aeneas.Std Result
open pedra_aeneas_si_kernel

theorem si_reader_beats_c_live :
    si_reader_beats true true false 0#u64 false false false 0#u64 = ok true := by
  unfold si_reader_beats
  rfl
