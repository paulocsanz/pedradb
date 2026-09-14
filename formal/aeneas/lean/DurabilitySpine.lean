-- Theorems over Aeneas extract of durability_spine_kernel.rs (RFC-0222 P0.7).
import Aeneas
import DurabilitySpineKernel
open Aeneas.Std Result
open pedra_aeneas_durability_spine_kernel
open durability_spine_kernel

/-- RFC-0222 P0.7 (atom `catalog:spine_replay`): the rustc-linked spine
replays ANY step sequence through the real WriteAckLedger (append /
barrier / ack) and asserts Inv-WAL after every step. Fate forall over
the extracted body. -/
theorem spine_replay_fate_iff :
    ∀ (l : write_ack_kernel.WriteAckLedger)
      (steps : Slice SpineStep),
      spine_replay l steps =
        (do
          let iter ←
            SharedSlice.Insts.CoreIterTraitsCollectIntoIteratorSharedIter.into_iter
              steps
          spine_replay_loop iter l) := by
  intro l steps
  unfold spine_replay
  rfl

/-- AS-IS dente: the barrier never runs — each append acks itself. -/
theorem spine_replay_as_is_fate_iff :
    ∀ (l : write_ack_kernel.WriteAckLedger)
      (steps : Slice SpineStep),
      spine_replay_as_is l steps =
        (do
          let iter ←
            SharedSlice.Insts.CoreIterTraitsCollectIntoIteratorSharedIter.into_iter
              steps
          let l1 ← spine_replay_as_is_loop iter l
          let t ← write_ack_kernel.WriteAckLedger.snapshot l1
          ok (t, l1)) := by
  intro l steps
  unfold spine_replay_as_is
  rfl
