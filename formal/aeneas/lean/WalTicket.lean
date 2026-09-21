-- Theorems over Aeneas extract of wal_ticket_kernel.rs (RFC-0230 P0.3).
import Aeneas
import WalTicketKernel
open Aeneas.Std Result
open pedra_aeneas_wal_ticket_kernel

/-- RFC-0193 / RFC-0230 (atom `catalog:reserve_frame`): a non-empty
    reserve returns ticket = current frontier and advances by `len`;
    empty len is a no-op. AS-IS never advances a distinct cursor. -/
theorem reserve_frame_fate_iff :
    ∀ (reserved_to len : U64),
      wal_ticket_kernel.reserve_frame reserved_to len =
        (do
          let b ← write_admission_kernel.batch_is_empty len
          if b then ok (reserved_to, reserved_to)
          else do
            let i ← lift (core.num.U64.saturating_add reserved_to len)
            ok (reserved_to, i)) := by
  intro reserved_to len
  unfold wal_ticket_kernel.reserve_frame
  rfl

theorem reserve_frame_as_is_never_advances :
    ∀ (reserved_to len : U64),
      wal_ticket_kernel.reserve_frame_as_is reserved_to len =
        ok (reserved_to, reserved_to) := by
  intro reserved_to len
  unfold wal_ticket_kernel.reserve_frame_as_is
  rfl

/-- Off-lock pwrite iff the env pin AND the handle is positional. -/
theorem pwrite_off_lock_fate_iff :
    ∀ (want can : Bool),
      wal_ticket_kernel.pwrite_off_lock want can =
        (if want then ok can else ok (false : Bool)) := by
  intro want can
  unfold wal_ticket_kernel.pwrite_off_lock
  rfl

theorem pwrite_off_lock_as_is_never :
    ∀ (want can : Bool),
      wal_ticket_kernel.pwrite_off_lock_as_is want can = ok (false : Bool) := by
  intro want can
  unfold wal_ticket_kernel.pwrite_off_lock_as_is
  rfl
