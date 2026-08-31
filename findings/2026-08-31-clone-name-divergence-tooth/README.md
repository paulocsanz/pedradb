# Clone name-divergence tooth (F-class audit clean, future hole closed)

Date: 2026-08-31. Ranked as the top unblocked slice by
`.grok/skills/verificacao-next/scripts/candidates.py`: board had no A
(absent twins), no B (data_fate freeze holes), no C (live_callers drift),
no D (0 cartoon plants — 38 inbound + 45 live_core), E complete 45/45;
F (clone drift) was the only actionable class.

## Audit result (2026-08-31, dirty tree, working-tree state)

All 7 clone groups audited fn-by-fn with the lint's own prod/test split
(`prod_fn_offsets`; test-module fns out of scope by design):

- unregistered shared **production** fns that are token-identical: **0**
  (the R6–R8 completeness tooth holds)
- unregistered shared production fns with token similarity ≥ 0.5
  (near-clones): **0**
- registered fns that drifted: **0**
- side-only fns (legitimate per-crate differences): raft
  `commit_kernel.recover_last_applied`, raft `ae_kernel.ae_f16_safe`,
  store `membership_kernel.plant_joint_schedule_ok`, 27 merge-only fns
  vs `iter_kernel`, 4 pin-only + 2 cursor-only fns.

A first-pass audit without the prod/test split flagged 17 "identical"
fns — all were `#[cfg(test)]` assertion clones across twin test modules
(e.g. `joint_election_old_majority_is_not_enough_during_add`), which the
clone teeth deliberately exclude. Recorded here so the number is not
re-discovered as a scare.

## The hole (future, not inhabited today)

`check_clones` fails closed on two corners: registered fn that drifted
("drifted (ta vs tb)"), and unregistered fn that is token-identical
("unregistered identical production clone fn(s)"). But a **new fn name
added to both sides of a pair with slightly different bodies** — a
near-clone that never was identical — passed silently forever. Same
fail-open shape as the suffix glob: exact-equality enforcement can only
see exact equality.

Timely because both sides of `membership_raft_store`
(`pedradb-raft/src/membership_kernel.rs` ↔
`pedradb-store/src/membership_kernel.rs`) are in the co-agent's dirty
set right now; a silent near-clone landing there would have been
invisible.

## Tooth (landed)

Name-level anti-silence in `check_clones`: every production fn NAME
shared across a clone pair must be either in `fns` (registered
identical) or in a new per-clone `diverged` object `{fn: reason}`.
Stale `diverged` entries (no longer a shared production fn) and
contradictions (fn in both `fns` and `diverged`) also fail. No
similarity threshold — the rule is name enrollment, mechanical, both
directions fail closed.

Current catalog needs zero changes: all 7 groups pass with no `diverged`
key (audit above). The key exists for the first real divergence someone
wants to record instead of unify.

## Verification

Throwaway fake-tree tests (`/tmp/clone_tooth_test.py`, 8 cases):
diverged-recorded pass, near-unrecorded FAIL, stale-diverged FAIL +
still-unlisted FAIL, both-listed FAIL, identical-unregistered still
caught (old tooth kept), side-only and test-module fns out of scope,
real repo 7/7 groups green. `pedra_formal.py --lint --clones` green on
the audit state.

Not claiming: that the 7 clone groups are semantically equivalent, or
that near-clones cannot exist under different names on the two sides —
the tooth binds shared names only.
