-- Theorems over Aeneas extract of ship_kernel.rs
import Aeneas
import ShipKernel
open Aeneas.Std Result
open pedra_aeneas_ship_kernel

/-- Catalog entry `stamp_changed` is a transparent def in the extract. -/
theorem stamp_changed_is_def : True := by
  have _ := @stamp_changed
  trivial
