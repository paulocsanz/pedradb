# 2026-09-14 — extract of rustc put/open scripts; public formal CI

**Not seL4. `db_rs_extracted` stays false (whole `db_kernel.rs` /
`concurrent_kernel.rs` not dumped). `R-rustc` stays never.**

## What happened

Public `formal-gates` was red: barrier TSV still named `db.rs`,
`ComposeM2.lean` had no `theorem` (lean_extracts), `verus_tcg_guest.sh`
still pointed at `tcg.rs`, aeneas-corpus exited if charon missing,
depth-floor selftest S1 used `live["close"]+1` (missed when registered
close > live close), ledger marker 322 vs catalog 324 after this land.

## Extract (RFC-0157 stage 2 — not a whole-file dump)

Aeneas will not eat 22k+9k lines at once. The rustc-linked handlers now
`match` named total fns extracted from `write_admission_kernel.rs`:

- `put_handler_plan` — `Db::put` → `put_with` → `apply_batch_with`
- `open_wal_head_plan` — `open_with_env_sourced` WAL-head (composes
  `torn_head_is_empty_log`); ConcurrentDb open still calls `Db::open_with_env`

`scripts/aeneas_write_admission.sh --required` restamped
`SOURCE.write_admission`. Theorems:
`put_handler_plan_fate_iff`, `open_wal_head_plan_fate_iff`
(`lake build WriteAdmission` green).

## Public CI (proof-check.yml)

- lean-extracts clones Aeneas `daa85d7` to `../aeneas` so lakefile
  `../../../../aeneas/backends/lean` resolves (Aeneas flake + mathlib).
- aeneas-corpus `nix build github:AeneasVerif/aeneas/daa85d7#aeneas`
  with hacl.cachix.org (their CI, `flake.nix` packages.aeneas links charon).

## Primary source

Aeneas `flake.nix` at pin `daa85d7e89400fa978be83fedbc7e475a83f0889`
(`packages.aeneas` postInstall `ln -s ${charon}/bin/charon $out/bin`).
