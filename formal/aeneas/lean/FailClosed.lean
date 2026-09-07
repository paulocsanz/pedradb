-- Theorems over Aeneas extract of fail_closed.rs (RFC-0002 / F102).
-- Charon --start-from parse_error_writes_status (rest is str/pattern).
import Aeneas
import FailClosedKernel
open Aeneas.Std Result
open pedra_aeneas_fail_closed_kernel

/-- Catalog entry: parse error writes a status line. -/
theorem parse_error_writes_status_true :
    parse_error_writes_status = ok true := by
  unfold parse_error_writes_status
  rfl

/-- AS-IS dente: parse error drops the socket mute. -/
theorem parse_error_writes_status_as_is_dente :
    parse_error_writes_status_as_is = ok false := by
  unfold parse_error_writes_status_as_is
  rfl
