# AppendEntries: committed entries are not overwritten

**Date:** 2026-09-07
**Primary source:** FVSquad, *Formal Verification Project Report* for
`dsyme/raft-lean-squad`, last updated 2026-04-28, commit `cd95f2c`.
Lean 4.30, 716 theorems, 0 `sorry`. Raw: `REPORT.md`.

## What the source actually says

End-to-end cluster safety (`fullProtocolStep_safe`, EL7) discharges
`hno_overwrite`: **committed entries are not overwritten**. That hypothesis
is CPS1 (`validAEStep_hno_overwrite`) via `h_committed_le_prev` + CT2, on a
valid AppendEntries step. The report is explicit: state-machine safety
depends on log-matching preservation across AppendEntries. This is not Pedra's
kernel; it is the class of guarantee F16 pays.

## Used this turn

Catalog pair `ae_entry` (`data_fate`, entry `ae_entry_action`): term conflict
at `entry_index <= commit_index` ⇒ `Refuse`, never `TruncateAndInstall`.
AS-IS truncates even at/before commit. Production `ae_kernel.rs` is now the
Verus term (`single_artifact`). Lemma
`lemma_mutant_violates_committed_conflict` is the second possibility. Pairs
`ae_ack` and `ae_f16_gate` keep their twins until their turns.

## Not claimed

Lean model of Pedra's full Raft. Dump of `handle_append_entries`.
raft-lean-squad's 716 theorems. “somos seL4”.
