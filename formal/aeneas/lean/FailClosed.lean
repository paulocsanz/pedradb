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

/-- F104: Transfer-Encoding is rejected. -/
theorem reject_transfer_encoding_true :
    reject_transfer_encoding = ok true := by
  unfold reject_transfer_encoding
  rfl

/-- AS-IS dente: TE is ignored. -/
theorem reject_transfer_encoding_as_is_dente :
    reject_transfer_encoding_as_is = ok false := by
  unfold reject_transfer_encoding_as_is
  rfl

/-- F105: a present bad int is an error. -/
theorem present_bad_int_is_error_true :
    present_bad_int_is_error = ok true := by
  unfold present_bad_int_is_error
  rfl

/-- AS-IS dente: bad int becomes the default. -/
theorem present_bad_int_is_error_as_is_dente :
    present_bad_int_is_error_as_is = ok false := by
  unfold present_bad_int_is_error_as_is
  rfl

/-- AS-IS dente: last Host wins (never conflicts). -/
theorem host_values_conflict_as_is_dente (a b) :
    host_values_conflict_as_is a b = ok false := by
  unfold host_values_conflict_as_is
  rfl

/-- AS-IS dente: empty Host counted as present. -/
theorem host_value_ok_as_is_dente (v) :
    host_value_ok_as_is v = ok true := by
  unfold host_value_ok_as_is
  rfl
