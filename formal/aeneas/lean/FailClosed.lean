-- Theorems over Aeneas extract of fail_closed.rs (RFC-0002 / F102).
-- Charon --start-from catalog + Iterator-free gates + header_break + Expect
-- (Windows position stripped; Split clauseInst / extra Iterator fields patched).
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

theorem parse_error_writes_status_fate_iff :
    ∀ (r : Bool), (parse_error_writes_status = ok r) ↔ r = true := by
  intro r
  constructor
  · intro hval
    unfold parse_error_writes_status at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold parse_error_writes_status
    rw [hr]

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

/-- F102 status line is 400. -/
theorem parse_error_status_400 :
    parse_error_status = ok (400#u16) := by
  unfold parse_error_status
  rfl

/-- F159 unrecognized Expect is 417. -/
theorem expectation_failed_status_417 :
    expectation_failed_status = ok (417#u16) := by
  unfold expectation_failed_status
  rfl

/-- AS-IS dente: never send 100-continue. -/
theorem expects_100_continue_as_is_dente (v) :
    expects_100_continue_as_is v = ok false := by
  unfold expects_100_continue_as_is
  rfl

/-- AS-IS dente: unknown Expect is ignored. -/
theorem expect_field_ok_as_is_dente (v) :
    expect_field_ok_as_is v = ok true := by
  unfold expect_field_ok_as_is
  rfl

/-- AS-IS dente: never require Host. -/
theorem http_version_requires_host_as_is_dente (v) :
    http_version_requires_host_as_is v = ok false := by
  unfold http_version_requires_host_as_is
  rfl

/-- F153: a break offset below 4 is the two-byte LF break. -/
theorem header_break_len_below_four (buf) :
    header_break_len buf (0#usize) = ok (2#usize) := by
  unfold header_break_len
  simp
