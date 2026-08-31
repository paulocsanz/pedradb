# New pure kernel landed outside the formal surface (leveling.rs)

Date: 2026-08-31. Commit `d99859d` ("core: leveled compaction + parallel
merge spans + point-lookup range prune") landed
`crates/pedradb-core/src/leveling.rs` — self-described "pure selection
kernel" for leveled compaction scheduling.

## What it is

`pub(crate)` pure decision functions on `LevelFile` slices:

- `leveled_enabled()` — `PEDRA_LEVELED` kill switch
- `level_target_bytes(level, l1_target)`
- `is_disjoint(files)` / `total_bytes(files)`
- `pick_l0_to_l1(...)` — L0→L1 job absorbing the disjoint overlap slice
- `pick_pushdown(...)` — bounded oldest-chunk pushdown one level down

Doc-comment invariants (hull closure, post-job disjointness by induction)
and in-file tests: `level_targets_scale_by_fanout`,
`disjoint_detection_rejects_stacked_runs`,
`l0_job_takes_only_the_overlapping_l1_slice`, `l0_job_respects_the_input_cap`,
plus a 200-case hull-disjointness property test (zero-padded keys).

## Why it is a formal-surface hole, not a style complaint

This kernel decides **where data lives** (compaction job selection → file
fate → what a lookup misses or hits; the same commit also prunes lookups
by chunk range). By the program's bar, a shipped-path kernel is either
frozen with a named plant or carries a registered residual row. It has
neither:

- `scripts/formal/catalog.json`: 0 references
- `scripts/formal/residuals.json`: 0 references
- no `_as_is` twin in the file
- no dst plant anywhere

## Why the teeth did not catch it

`pedra_formal.py` discovers kernels by filename pattern
(`crates/*/src/**/*_kernel.rs`, line ~519). `leveling.rs` does not match,
so `kernel_files` stayed 43, the completeness tooth (RFC-0157 R8) never
scanned it, and `candidates.py` (whose A-class reads catalog holes, not
the tree) cannot surface it either. The blind spot is **structural**: any
future pure kernel named without the `_kernel.rs` suffix lands unenrolled
the same way.

## Options (next round's call)

1. Enroll it as a normal slice: twin `as_is` + dst plant + freeze entry
   (B-class freeze hole on landed code).
2. Extend the lint's kernel discovery to a content/pattern-independent
   list so a suffix rename cannot silently unenroll a kernel (same
   fail-closed class as the seed and kill-target echoes: the registry
   should not depend on a naming convention nobody enforces).
3. Both — 2 is cheap and closes the structural hole for every future
   kernel; 1 is the real verification work for this one.

Not claiming: that leveling.rs is wrong, or that its property tests are
insufficient. The finding is enrollment, not a bug report.

## Reading

Same root-cause class as the seed collapse: a convention doing silent
work — `.unwrap_or()` picked a world when parsing failed; here a filename
suffix decides what counts as a kernel. Both fail open; both need the
registry to be explicit and the miss loud.

## Post-note (same day): option 2 landed — marker⇔registry tooth

`scripts/formal/pedra_formal.py` + `scripts/formal/residuals.json` now
implement convention-independent kernel discovery:

- `glue.kernel_paths` — explicit 44-entry enrollment list (43 glob +
  `leveling.rs`). Discovery is **glob ∪ registry**
  (`decision_kernel_paths`), so a suffix rename can no longer silently
  unenroll a kernel: every glob match absent from the registry is a FAIL.
- Suffix-less registry entries must carry a `//! kernel:` marker line
  (present in `leveling.rs`); a marked file not enrolled is a FAIL. Both
  directions fail closed.
- `TCB_FREEZE_ALLOWLIST` carries `leveling.rs` as an explicit
  transitional state ("pending pair+twin"), printed loud on every lint
  run; it leaves the allowlist when the catalog pair lands.
- `glue.kernel_files` 43→44, `glue.kernel_loc` → 11678 (live recompute
  on the 2026-08-31 dirty tree; the co-agent's 8 kernel WIP files are
  still dirty, so kernel_loc/handler_loc take one more re-freeze when
  their tree resolves — `handler_loc` stays drifted (83217 vs live
  86242) and is theirs, not touched here).

Tooth verified standalone on a fake mini-tree (throwaway
`/tmp/tooth_test.py`, 10 cases): marked-unenrolled FAIL,
enrolled-unmarked FAIL, glob-unenrolled FAIL, enrolled-missing FAIL,
no-registry FAIL, duplicates FAIL, complete pass, discovery-union pass,
discovery-drops-missing pass, real-repo pass (44 kernels, 1 suffix-less).

Word-based detection was measured and rejected: 6 non-suffix files say
"kernel" in a leading `//!` block (leveling + 5 false positives), so a
word grep is advisory only; the marker⇔registry tooth is the mechanical
truth.

Option 1 (twin + plant for leveling) remains open, blocked on the
co-agent's `three_teeth_queued.rs` (plant infrastructure) being clean at
HEAD.

## Tree sweep (2026-08-31 evening, advisory — was leveling the only one?)

The enrollment tooth enforces; it does not discover. This sweep answers
whether any OTHER pure decision kernel sits outside the surface.
Method (persisted as `pure_sweep.py` in this dir, re-runnable):

- surface = glob `*_kernel.rs` (non-verus) ∪ `glue.kernel_paths` ∪ every
  file referenced by catalog.json (pair kernel/twin/plant/callers, clone
  sides) = **148 files**
- sweep every other non-verus `.rs` under `crates/*/src`, prod code only
  (before `#[cfg(test)]`), pure shape: no `self` receiver, no
  unsafe/IO/thread/lock/atomics/async signals in the body, non-`()`
  return; brace-matched bodies, comments stripped

~46 leftover files, all classifiable by hand:

- codecs/framing (`tcp.rs`, `persist.rs`, `dcs`, `msg.rs`, `fdb_layers`,
  `change_feed`, `sql`), bench/CLI/soak harness (`rocksdb-parity-bench`,
  `cli`, `montanha-*`, `world` bins, `dst` bins), fault injection
  (`buggify_hooks`, `pct_hooks`), DST scheduling (`world/schedule.rs`,
  `pct*`, `scheduler.rs`), test oracles (`oracle`, `sim`), plumbing
  (`tls`, `client.rs`, `backup`, `txn`, `knobs`, `fold`)
- **`pedradb-core/src/verified.rs`** — RFC-0058 verified-profile
  declaration: not a data-fate kernel; it is the claims meta-surface and
  carries its own machine tie to the catalog
  (`verified_report_matches_catalog`; ON set must equal catalog pair ids)
- **`pedradb-core/src/wal/recover_choose.rs`** — DST injection harness by
  its own header ("not a second recover policy"; production calls
  `recover_kernel`, these only tear/flip/forge WAL images to drive the
  real reader)
- `sst/table.rs` helpers (`block_target` etc.) live in a catalog caller
  (handler side of `cf_kernel`), tracked by the handler_loc freeze, not
  kernel-side

**Conclusion: leveling.rs was the only unenrolled pure data-fate
decision kernel in the tree.** The enrollment surface is complete at the
pure-kernel level as of this sweep.

Not claiming: that the heuristic is exhaustive (a decision fn taking
`&self` on a pure struct, or returning `()`, would be missed — advisory
only, per the word-grep rejection above); anything about codec/harness
correctness (plants and oracles exercise those); that clones are
semantically equivalent.
