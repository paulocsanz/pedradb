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

## Close (same day, later): option 1 landed — catalog pairs `leveling` + `leveling_pick`

Unblocked by putting the plant in the kernel file itself (the
`<entry>_on_live_…_is_not_ok` convention over `LevelFile` inputs, no Db and
no `three_teeth_queued.rs` dependency):

- **Pair `leveling` (close, entry `level_target_bytes`)** — twin
  `verus/leveling.rs` (6 verified, 0 errors): exec == spec under the
  production no-saturation bound, ladder monotonicity lemma. AS-IS
  `level_target_bytes_as_is` (no exp cap, wrapping): deep-level target
  wraps downward. Plant `level_target_bytes_on_live_deep_level_is_not_ok`.
- **Pair `leveling_pick` (atom, atom `pick_l0_to_l1_model`, entry
  `pick_l0_to_l1`)** — twin `verus/leveling_pick.rs` (23 verified, 0
  errors): hull-returning model (sel, hull_lo, hull_hi, slice) with the
  slice EXACTLY the recursive `overlapping_prefix` over the hull;
  `lemma_prefix_excludes` (non-overlapping dst file never enters the
  prefix); pushdown model `None` exactly on empty source or non-disjoint
  dst; mutants whole-level / uncapped / blind, one verified divergence
  each. Plant `pick_l0_to_l1_on_live_slice_is_not_ok` (all three dentes).
- Runners split `verus_leveling.sh` + `verus_leveling_pick.sh` (one SRC=
  per script — the scripts tooth requires it; also the fix for the
  cross-file Verus interference below).
- `TCB_FREEZE_ALLOWLIST` emptied (graduation comment left in place);
  `glue.kernel_loc` re-frozen 11678 → 11790 (44 kernels, count unchanged).
  `handler_loc` stays 83217 frozen (the drift line is the co-agent's
  db.rs WIP, theirs to re-freeze).

### Verus interference note (kept for the next multi-pair twin)

One combined twin file did NOT verify: the mere declaration of the
recursive spec fn `overlapping_prefix` broke the unrelated
`level_target_bytes` (nonlinear arithmetic) query. Not opaque-able, not
resource-bound (`--rlimit 200` no help, ~0.5–1 s runs); root cause inside
Verus/Z3 query perturbation unidentified. Resolution: file split per pair
(matches the catalog's one-twin-file-per-pair shape). Do not re-litigate;
keep the split.

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

## Re-sweep (same day, night) on the 11-file co-agent WIP tree

Re-ran `pure_sweep.py` on the dirty tree (pre-landing intelligence for
their round) plus a fn-level diff of every dirty file vs HEAD:

- **No new pure data-fate surface.** New fns in the whole WIP: 2 diag
  knobs (`wal/mod.rs::walfd_diag_enabled`,
  `pedradb-posix::fdsync_diag_enabled` — plumbing/knob class, same as
  `PEDRA_BLOCK_TARGET`) and one private refactor
  (`pedradb-posix::fdatasync_file_inner`). 0 new public fns in any
  enrolled kernel file.
- The fdatasync refactor is in the durability path: guard
  `posix_unsafe_rc_sites_all_gated` re-run green on the WIP tree; the
  `fdatasync_rc` twin/token teeth are file-side and unaffected.
- **Blind spot found (methodological, open):** the surface is FILE-level
  (glob ∪ registry ∪ catalog). A NEW pub decision fn added inside an
  already-enrolled kernel file is invisible to the sweep (file excluded)
  AND to the teeth (token tooth checks only cataloged entries). Hand-
  checked clean this time (0 new pub fns in the 11 dirty files); a
  fn-level tooth (every pub fn in an enrolled kernel must be an entry,
  an as_is, a clone fn, or explicitly allowlisted) is the hardening
  candidate for a future round.

## Delta re-audit (2026-08-31 later night): WIP drifted while landing

The 11-file WIP kept growing after `97c4593` (214 → 310 insertions
between two `git diff --stat` snapshots minutes apart — the co-agent is
actively landing). Re-ran the fn-level delta check on the new state:

- 3 genuinely new private fns since the first re-audit: 2 in `db.rs`
  (`write_merged_with_cf` :993, `write_imm_l0_files_inner` :5047 —
  compaction I/O writers, plumbing class; `db.rs` is not an enrolled
  kernel file, `db_rs_extracted=false`) and 1 in `wal/mod.rs`
  (`sync_data_inner` :275 — inside the blind spot's file class).
- `sync_data_inner` is an order-preserving extraction: the durability
  sequence `write_pending_frame` → `flush` → `sync_data{,_strong}` moved
  verbatim out of `sync_data` so a timing wrapper can measure it; the
  wrapper returns the `Result` unchanged (no error swallowing).
  `set_full_fsync`/`full_fsync` are pre-existing pub fns (RFC-0036).
- `eintr_then_late_cqe` (cqe_kernel.rs) is a signature reformat
  (multi-line → one line), not a new fn.
- Still **0 new pub fns** in any enrolled kernel file. Guard
  `posix_unsafe_rc_sites_all_gated` re-run green on the drifted WIP
  (pedradb-posix grew to +34 in the delta).
- Second data point for the open blind spot: file-level surface clean,
  hand-check required again. The fn-level tooth remains the fix; until
  it lands, every co-agent delta costs one manual fn-level diff.

## Close (2026-08-31, same night): fn-level tooth landed

Third consecutive delta hand-check in one landing was the trigger.
`pedra_formal.py::check_kernel_fn_surface` + `glue.kernel_fn_allowlist`
(residuals.json) now enforce the fn-level surface, fail-closed on
CHANGE, for every enrolled kernel (glob ∪ registry):

- classified = catalog entry (pair on this kernel) | catalog `as_is`
  (exact) | `_as_is` naming convention | `_spec` kernel-side spec twin |
  catalog clone fn (side on this file) | explicit baseline entry;
- FAIL: unclassified pub/pub(crate) fn (the exact blind-spot scenario —
  a `set_parallel_jobs` analog in an enrolled file is now loud);
- FAIL: stale baseline entry (fn gone) — the baseline only shrinks;
- FAIL: baseline entry that auto-classifies (redundant), baseline key on
  a non-enrolled file, missing `glue.kernel_fn_allowlist` entirely.
- Harness (throwaway /tmp/fntooth_test.py, mini-tree like the marker
  tooth's): 14/14 — clean pass, unclassified method/pub(crate)/marker-
  file fn FAIL, allowlist pass, stale FAIL, redundant FAIL, wrong-key
  FAIL, missing-key FAIL, `_as_is`/clone auto-pass, comment mentions
  don't count, mid-WIP-style new fn FAIL.

### The steady-state finding the tooth itself surfaced

Measuring before building: **425 pub/pub(crate) fns in the 44 kernels;
326 auto-classify; 93 in 29 files did not.** The blind spot was not a
hypothetical delta risk — it existed in steady state. The 93 include
genuine uncataloged decision fns, e.g. `leveling.rs::pick_pushdown`
(pushdown job selection: modeled inside the `leveling_pick` twin but
never a pair/entry/plant of its own), `txn_kernel.rs`'s eleven
(`revert_user_action`, `leftover_txn_is_aborted`,
`prepare_error_aborts_earlier`, …), the six `cf_kernel.rs`
encode/decode/decision fns, `cqe_kernel.rs`'s `cqe_act`.

The baseline (date-stamped 2026-08-31) is a WORKLIST, not an absolution:
every entry must graduate to a real class — enroll as a catalog pair,
codec/knob/api class, or a named catalog-gap finding — and the tooth
enforces that the list can only shrink. Same transitional shape as
`TCB_FREEZE_ALLOWLIST` before the leveling pair landed.

Not claiming: that the 93 are wrong, or that classification is
verification. The tooth proves every pub fn in an enrolled kernel is
NAMED; each graduation round is where the verification claim (or the
published residual row) gets made.

## Graduation round 1 (same night): `pick_pushdown` → pair `leveling_pushdown`

First baseline entry graduated, chosen because the verification
artifacts already existed: the `leveling_pick` twin proves
`pick_pushdown_model` (`None` exactly on empty source or non-disjoint
dst; blind-mutant divergence) — only the catalog shape was missing.

- Catalog pair `leveling_pushdown` (atom, atom `pick_pushdown_model`,
  entry `pick_pushdown`, as_is `pick_pushdown_as_is_blind`,
  `token_src=pick_pushdown_as_is_blind` so the token tooth diffs the
  mutant mirror that exists on BOTH sides — a real dente, not
  twin-only). Shares twin + runner with `leveling_pick` (one Verus run
  verifies both models).
- Dedicated plant `pick_pushdown_on_live_pushdown_gate_is_not_ok`
  (empty-src None; disjoint dst takes the OLDEST chunk and exactly its
  overlapping slice; stacked dst refused by entry, rewritten by the
  blind mutant). Module 9→10 tests, 10 passed.
- Board row `on!("leveling_pushdown", …)`; baseline 93→92 (leveling.rs
  keeps `is_disjoint`, `leveled_enabled`, `overlaps`, `total_bytes`).

Also this round: co-agent landed `6d6168c` (parallel compaction;
`leveling.rs::overlaps` promoted to `pub(crate)` — already inside the
baseline because the promotion was in the worktree at baseline-build
time, so the tooth stayed green through the landing; graduation
candidate for a later round alongside `is_disjoint`).
