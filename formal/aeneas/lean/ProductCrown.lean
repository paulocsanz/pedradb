-- Theorems over Aeneas extract of product_crown_kernel.rs (RFC-0222 P0.7).
import Aeneas
import ProductCrownKernel
open Aeneas.Std Result
open pedra_aeneas_product_crown_kernel
open product_crown_kernel

/-- RFC-0222 P0.7 (atom `catalog:product_crown`): the rustc-linked crown
folds d1_modelo ∧ (legal cut ⇒ d1_holds) over every torn cut of a
ledger. Fate forall over the extracted body. -/
theorem product_crown_fate_iff :
    ∀ (s : wal.wal_state_kernel.WalState),
      product_crown s =
        (do
          let flags ← acked_flags s
          let m ← env_crash_kernel.CrashModel.of s.written s.synced
          let last ← lift (core.num.U64.saturating_add s.written 2#u64)
          product_crown_loop s.acked s.synced s.written flags m last 0#u64) := by
  intro s
  unfold product_crown
  rfl
