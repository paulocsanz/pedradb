-- Theorems over Aeneas extract of batch.rs (RFC-0150 P2a).
-- Charon --start-from write_record_count_ok (decode has early-return-in-loop).
import Aeneas
import BatchKernel
open Aeneas.Std Result
open pedra_aeneas_batch_kernel

/-- Catalog entry: prefix count is not Ok. -/
theorem write_record_count_ok_prefix :
    batch.write_record_count_ok (3#u32) (2#usize) = ok false := by
  unfold batch.write_record_count_ok
  have h : UScalar.cast UScalarTy.Usize (3#u32) = 3#usize := by native_decide
  simp [h, lift]

/-- AS-IS dente: prefix still admits. -/
theorem write_record_count_ok_as_is_dente :
    batch.write_record_count_ok_as_is (3#u32) (2#usize) = ok true := by
  unfold batch.write_record_count_ok_as_is
  rfl
