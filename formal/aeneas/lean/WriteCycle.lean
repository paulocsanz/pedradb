-- Theorems over Aeneas extract of write_cycle_kernel.rs (RFC-0222 P0.7).
import Aeneas
import WriteCycleKernel
open Aeneas.Std Result
open pedra_aeneas_write_cycle_kernel.write_cycle_kernel

/-- RFC-0222 P0.7 (atom `catalog:serial_cs_ns`): serial CS is the saturating
sum of encode+write+guard+lock+insert+publish+grp. Fate forall over the
extracted body. AS-IS is the fire-120 constant 2200 ns. -/
theorem serial_cs_ns_fate_iff :
    ∀ (p : WritePhaseNs),
      serial_cs_ns p =
        (do
          let i ← lift (core.num.U64.saturating_add p.wal_encode p.wal_write)
          let i1 ← lift (core.num.U64.saturating_add i p.mem_guard)
          let i2 ← lift (core.num.U64.saturating_add i1 p.mem_lock)
          let i3 ← lift (core.num.U64.saturating_add i2 p.mem_insert)
          let i4 ← lift (core.num.U64.saturating_add i3 p.publish)
          ok (core.num.U64.saturating_add i4 p.grp)) := by
  intro p
  unfold serial_cs_ns
  rfl

/-- AS-IS dente: CS is a constant 2200 ns. -/
theorem serial_cs_ns_as_is_constant :
    ∀ (p : WritePhaseNs),
      serial_cs_ns_as_is p = ok 2200#u64 := by
  intro p
  unfold serial_cs_ns_as_is
  rfl
