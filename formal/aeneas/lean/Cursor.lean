-- Theorems over Aeneas extract of cursor_kernel.rs
import Aeneas
import CursorKernel
open Aeneas.Std Result
open pedra_aeneas_cursor_kernel

theorem next_seq_from_zero :
    next_seq 0#u64 = ok (1#u64) := by
  unfold next_seq
  have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
  simp [h]
