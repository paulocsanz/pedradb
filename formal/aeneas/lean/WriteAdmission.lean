-- Theorems over the Aeneas extract of production write_admission_kernel.rs
-- (RFC-0170 P2.1). Fail-closed: this file must not contain a hole.
import Aeneas
import WriteAdmissionKernel
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel

/-- Extracted idle gate is true iff every stall knob is off. -/
theorem write_admission_idle_matches_spec :
    write_admission_idle false false false = ok true := by
  unfold write_admission_idle
  rfl

/-- A live mem-stall knob refuses the idle path. -/
theorem write_admission_idle_mem_stall_refuses :
    write_admission_idle true false false = ok false := by
  unfold write_admission_idle
  rfl

/-- AS-IS dente: stall knobs are ignored (always idle). -/
theorem write_admission_idle_as_is_dente :
    write_admission_idle_as_is true true true = ok true := by
  unfold write_admission_idle_as_is
  rfl
