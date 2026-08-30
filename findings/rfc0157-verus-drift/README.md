# RFC-0157 — verus_check --all: 3 twins fail to compile under the pinned toolchain

- date: 2026-08-30  pinned: `release/0.2026.08.23.fbbbbcf` (arm64-macos binary,
  self-reported `0.2026.08.09.92f466f`)
- command: `./scripts/formal/verus_check.sh --all`
- result: **53 pass / 3 fail** (56 twins total)

| twin | failure | shape |
|------|---------|-------|
| `cqe_res` | `error[E0308]: mismatched types` ×3 | toolchain API drift |
| `fdatasync_rc` | `error[E0308]: mismatched types` ×2 | toolchain API drift |
| `journal_pin` | `error: cannot call function `pin::fold_pins_on_read` with mode exec` | exec/spec mode drift |

## Reading

- The three failures are **compile** failures (Verus-language drift), not
  verification failures: the twins were written against an older Verus
  surface and no longer type-check under the pinned release.
- The default first set (P0.1: `l28`, `group_commit`, `sst_crc_fate`) is
  green under the same pinned toolchain — the P0 bar (one twin
  machine-checked end-to-end) is met and exceeded.
- Fixing the three drifted twins is **P2.1** (corpus Verus expandido).
  Until then `verus_check.sh --all` reports them FAIL and exits nonzero —
  by design: the aggregator does not round a compile-drifted twin to ok.
- R-verus stays in `never_floor`: this note is itself evidence that the
  pinned toolchain is a moving dependency.
