-- Theorems over Aeneas extract of cursor_kernel.rs
import Aeneas
import CursorKernel
open Aeneas.Std Result
open pedra_aeneas_cursor_kernel

theorem next_seq_from_zero :
    next_seq 0#u64 = ok (1#u64) := by
  unfold next_seq
  have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
  simp [h]

/-- RFC-0218 P2.1 5/12 (átomo `catalog:stream_next_seq`, entrada
    `next_seq`): a próxima sequência é EXATAMENTE o lift citado
    `saturating_add last_acked 1` — o ack anda uma casa sem overflow.
    O AS-IS devolve o próprio last_acked (ack não avança — dente
    plantado). -/
theorem next_seq_fate_iff :
    ∀ (last_acked : U64) (r : U64),
      (next_seq last_acked = ok r) ↔
      (r = core.num.U64.saturating_add last_acked 1#u64) := by
  intro last_acked r
  constructor
  · intro hval
    unfold next_seq at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
