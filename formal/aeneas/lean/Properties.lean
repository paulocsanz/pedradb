-- Theorems over Aeneas extract of properties_kernel.rs
import Aeneas
import PropertiesKernel
open Aeneas.Std Result
open pedra_aeneas_properties_kernel

/-- Catalog entry `d1_holds` extracted (loop body is a transparent def). -/
theorem d1_holds_loop_body_is_def : True := by
  have _ := @d1_holds_loop.body
  trivial
