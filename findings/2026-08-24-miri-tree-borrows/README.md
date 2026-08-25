# RFC-0052 P1.3 — Stacked Borrows vs Tree Borrows (2026-08-24)

Same two `pedradb-sim` DST tests as `scripts/miri_dst_smoke.sh`, rerun under
`-Zmiri-tree-borrows` (`scripts/miri_dst_tree_borrows.sh`).

Host: Darwin aarch64, miri 0.1.0 (7608eb7 2026-08-05).

| Model | `crash_after_sync_recovers_committed` | `failing_env_nth_put_then_reopen_recovers_prefix` |
|---|---|---|
| Stacked Borrows | ok | ok |
| Tree Borrows | ok | ok |

**No divergence.** RFC-0052 P1.3 says promote Tree Borrows to *required* CI
only if the two models disagree once. They did not — the TB script stays
optional (not a `MIRI_REQUIRED` job).

Darwin notes (not aliasing): Miri cannot `fcntl(F_PREALLOCATE)`; under
`cfg(miri)` `preallocate_file` no-ops like Linux. Process-lifetime TLS /
std `OnceBox` need `-Zmiri-ignore-leaks` on both scripts (same flag, both
models).

Mutant: `MIRIFLAGS=-Zmiri-disable-isolation bash scripts/miri_dst_tree_borrows.sh`
exits 1 (refuses a Stacked Borrows rerun).
