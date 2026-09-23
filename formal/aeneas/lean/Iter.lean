-- Theorems over Aeneas extract of iter_kernel.rs
import Aeneas
import IterKernel
open Aeneas.Std Result
open pedra_aeneas_iter_kernel

theorem iter_window_keep_live :
    iter_window_keep true = ok true := by
  unfold iter_window_keep
  rfl

theorem iter_window_keep_as_is_dente :
    iter_window_keep_as_is false = ok true := by
  unfold iter_window_keep_as_is
  rfl

/-- RFC-0213 P1.2 (storage cadence, atom `catalog:iter_window`): the
    compat iterator window keeps an entry EXACTLY when its snapshot is
    still live — the identity over the extracted body (fate forall,
    RFC-0170 P2.4). The AS-IS mutant keeps everything, resurrecting
    entries hidden after the snapshot died (the lie the DST plant
    `iter_window_keep_on_live_hidden_is_not_ok` refutes). -/
theorem iter_window_keep_fate_iff :
    ∀ (snapshot_live : Bool) (v : Bool),
    (iter_window_keep snapshot_live = ok v) ↔ v = snapshot_live := by
  intro snapshot_live v
  unfold iter_window_keep
  cases snapshot_live <;> cases v <;> simp
