-- Theorems over Aeneas extract of cqe_kernel.rs (U1 / F203).
-- RUSTFLAGS=--cfg test so as_is mutants are visible; Atomic telemetry in
-- submit_complete_act stripped in aeneas_cqe.sh.
import Aeneas
import CqeKernel
open Aeneas.Std Result
open pedra_aeneas_cqe_kernel

theorem cqe_res_ok_nonneg :
    cqe_res_ok 0#i32 = ok true := by
  unfold cqe_res_ok
  simp

/-- Harvested CQE is used even if submit returned Err. -/
theorem submit_complete_act_harvested :
    submit_complete_act false true = ok SubmitCompleteAct.UseHarvested := by
  unfold submit_complete_act
  rfl

/-- AS-IS dente: failed submit returns Err (releases the buffer). -/
theorem submit_complete_act_as_is_dente :
    submit_complete_act_as_is false false =
      ok SubmitCompleteAct.ReturnSubmitErr := by
  unfold submit_complete_act_as_is
  rfl

/-- AS-IS dente: constant per-opcode tag. -/
theorem next_user_data_as_is_dente :
    next_user_data_as_is (1#u64) (0x77#u64) = ok (0x77#u64, 1#u64) := by
  unfold next_user_data_as_is
  rfl
