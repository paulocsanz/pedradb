-- Theorems over Aeneas extract of pedradb-posix (RFC-0073).
-- Charon --start-from fdatasync_rc_ok (rest of lib.rs is syscall/unsafe).
import Aeneas
import PosixKernel
open Aeneas.Std Result
open pedra_aeneas_posix_kernel

/-- Catalog entry: rc 0 is Ok. -/
theorem fdatasync_rc_ok_zero :
    fdatasync_rc_ok (0#i32) = ok true := by
  unfold fdatasync_rc_ok
  rfl

/-- Catalog entry: nonzero rc is not Ok. -/
theorem fdatasync_rc_ok_nonzero :
    fdatasync_rc_ok (5#i32) = ok false := by
  unfold fdatasync_rc_ok
  rfl

/-- AS-IS dente: nonzero still admits. -/
theorem fdatasync_rc_ok_as_is_dente :
    fdatasync_rc_ok_as_is (5#i32) = ok true := by
  unfold fdatasync_rc_ok_as_is
  rfl
