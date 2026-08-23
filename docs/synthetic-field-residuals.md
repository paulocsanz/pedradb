# Synthetic field residuals (RFC-0020 P2.2–P2.5)

**Updated:** 2026-08-13  
**Parent:** [RFC-0020](rfc/0020-synthetic-field-maturity.md)

## Weekly LEDGER triage checklist (P2.2)

Run weekly (or after overnight soak):

1. Collect `volume_report*.json` and explore logs from `scripts/overnight_soak.sh` / `stretch_soak.sh` / `explore_campaign.sh`.
2. If any `silent_wrong != 0` or gate failure: open a REAL in `../determinismo/pedradb-dst/findings/LEDGER.md` (same session as discovery).
3. Capture repro: seed/offset, command line, pedradb git SHA, report JSON.
4. Fix in pedradb (or store) + add regression test/seed; mark FIXED with link.
5. Re-run `scripts/ci_silent_wrong_gate.sh` and the failing soak mode.
6. Note open REALs older than 14 days for prioritization.

## Linux det_io / QEMU residual (P2.3) — Universe E

| Entry | Location | Status |
|-------|----------|--------|
| **pedradb wrapper** | `scripts/det_io_status.sh` | invokes sibling `linux_det_io_ci` + `blocked_residuals_status` |
| det_io smoke | `../determinismo/pedradb-dst/scripts/det_io_smoke.sh` | **entry exists**; hard CONTRACT-OK needs Linux runner |
| det_io proof | `../determinismo/pedradb-dst/scripts/det_io_proof.sh` | entry exists |
| Linux CI entry | `../determinismo/pedradb-dst/scripts/linux_det_io_ci.sh` | Darwin → PRELOAD-MISS / RecordingEnv substitute (exit 0); Linux → CONTRACT-OK hard |
| QEMU subset | `../determinismo/pedradb-dst/scripts/qemu_subset_revalidate.sh` | entry exists; **no guest image** → residual |

**Residual ticket (honest):** full det_io CONTRACT-OK and QEMU guest revalidation are **blocked** without Linux CI runner + optional guest image. In-tree FailingEnv / RecordingEnv remain the authoritative disk-fault proof on macOS and default CI.

```bash
bash scripts/det_io_status.sh /tmp/e-detio.txt
bash scripts/universe_abcde.sh   # A–E one-shot
```

## Honesty bench (P2.4)

```bash
cargo bench -p pedradb-core --bench baseline
```

Sync-labeled criterion baseline; not a field-parity claim. Optional in CI.

## Miri on unsafe crates (P2.5)

Gate: `scripts/miri-unsafe-islands.sh` (`MIRI_REQUIRED=1` in CI).

| Island | What Miri runs | Residual |
|--------|----------------|----------|
| `pedradb-posix` | all tests, `MIRIFLAGS=-Zmiri-disable-isolation` (real `fdatasync` FFI) | — |
| `pedradb-io-uring` | `cqe_kernel` (unique tag / harvest) | `IoUringEnv` ring syscalls need a Linux kernel |
| `pedradb-capi` | `handles` (slot+generation) | `StoreCluster` C tests are FS + `!Send` |
| `pedradb-core` | not required (`forbid(unsafe_code)`) | — |

```bash
bash scripts/miri-unsafe-islands.sh
# CI:
MIRI_REQUIRED=1 bash scripts/miri-unsafe-islands.sh
```

Local host without nightly+miri prints `MIRI_RESIDUAL` and exits 0 (no false green in CI).

## Race job (P1.4)

```bash
bash scripts/race_job.sh
# optional TSan:
PEDRA_RUN_TSAN=1 bash scripts/race_job.sh
```

Default job: `concurrent_race_stress` multi-thread test. TSan is optional nightly.
