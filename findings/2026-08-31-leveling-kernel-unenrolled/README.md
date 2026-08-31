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
