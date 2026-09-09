# Lock-free seL4 price blocked at Aeneas raw-pointer deref (fire 732)

**Question.** `crates/pedradb-lockfree/src/lib.rs` (the `LfStack` the
RFC-0055 async WriteThread join links) has zero formal coverage. What is
the seL4-class price on its rustc bodies, and where does the pinned
toolchain stop?

**Method.** Standard kernel-extract recipe, no stand-in: harness crate
`formal/aeneas/lockfree-kernel` whose `[lib] path` is the production
file, extracted by `scripts/aeneas_lockfree.sh` (charon `0.1.232` →
aeneas `daa85d7`, same pins as every other kernel).

**Result — Charon half works.** `charon cargo --preset=aeneas` extracts
the whole unsafe crate (CAS loop, swap-walk, raw-pointer borrows) with
zero errors: `formal/aeneas/out/lockfree_kernel.llbc`.

**Result — Aeneas half rejects the four bodies that matter.**
`Aeneas does not yet support dereferencing raw pointers` → bodies
emitted as `sorry`:

| body | rustc span | fate |
|---|---|---|
| `LfStack::push` (CAS elect leader) | lib.rs 71:4–88:5 | `sorry` |
| `LfStack::take_all` (swap + walk + reverse) | lib.rs 93:4–105:5 | `sorry` (deref at 99:28) |
| `StolenHandles::get` | lib.rs 129:4–137:5 | `sorry` |
| `StolenHandles::relink_from` | lib.rs 141:4–149:5 | `sorry` (deref at 146:36) |

Translated with real bodies: `LfNode::new/data`, `LfStack::new/
is_empty`, `StolenHandles::len/is_empty`, `Default`, `Drop`. The
algorithm-carrying bodies are exactly the blocked ones; the translated
ones are the plumbing (theorems over them alone would be the banned
`is_empty` wrap shape).

**Blocker location (primary source read).** Pinned checkout
`/Users/paulo/software/aeneas` @ `daa85d7`, `src/interp/InterpPaths.ml`
line 116: `Deref, _, TRawPtr _` raises explicitly — the comment above
says a raw pointer symbolic value "can't" be expanded. Not a bug: an
unimplemented feature. Upstream check (2026-09-09): `origin/main` is 89
commits ahead of the pin and **none** touch raw-pointer deref, so a pin
bump alone does not unblock this.

**Script behavior is honest.** `scripts/aeneas_lockfree.sh` exits 1 on
the partial extract and writes **no** `SOURCE.lockfree` stamp (verified:
`ls formal/aeneas/out/SOURCE.lockfree` → No such file). No sorry-stamped
extract was committed to the lake project; nothing overclaims payment.

**What would pay it, in honesty order:**
1. Upstream Aeneas raw-pointer deref support lands → bump pin, re-run
   the script, stamp, add `LockfreeKernel.lean`+`Lockfree.lean` to the
   lake project. The harness+script committed here are the standing
   gate: they go green the day the tool can pay.
2. Verus on the **same** `AtomicPtr<LfNode<T>>` types (atomic CAS loop
   linearizability) — separate dedicated effort, not fire-sized.
3. Banned (user-binding): toy index-model stand-in twin, sorry-stamped
   stamp, wrap theorems over `is_empty/len` plumbing.

**Verification state of the crate itself (asked by user 2026-09-09):**
unit tests 4/4 green; the two RFC-0055 join-protocol tests in core
(`rfc0055_pipeline_four_writers_async_visible`,
`async_four_writers_join_leader_not_bypass`) pass on the working-tree
pipeline pair; formal: blocked as above. Perf (Linux p149b, peer
`sync=false`): v3 wake-after-WAL min ratio 0.506 vs KEEP 0.729 — see
`findings/2026-09-09-lockfree-join-v3-wake-after-wal.md`.
