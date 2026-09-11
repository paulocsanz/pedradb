# 2026-09-10 — RFC-0192 P0: write-cycle kernel + WRITEPHASE `cut=` + CLI

Fire-120 `cut=lock_hold` 500 ns / `as_is=2200` is the AS-IS dente, not
the ranking. P0 lands the integer kernel and the dump that calls it.

## Kernel (`write_cycle_kernel.rs`)

Linux quiet 0189 P0.1 ns/op pin (`LINUX_QUIET_0189_P01`):

```
enc=350 wr=890 guard=2230 mlock=380 mins=580 publish=820 grp=270 lock_wait=2460
```

- `name_cut` → `mem_guard` (max structural slice; `lock_wait` is not a candidate).
- `name_cut_as_is` → `lock_hold` (the generator, always).
- After `guard=0` → `wal_write` (0189 P1.2 if a quiet remeter still names it).
- Predicted lock_wait: `(L-1)/L · CS`; `L=1` ⇒ 0. Ceiling, not cartaz QPS.
- Lane collapse: `max * lanes > 2 * total` (twice fair share).

`write_cycle_forecast` is the table `pedra scale-model write` prints
verbatim (`rfc0192_pedra_scale_model_write_prints_kernel`).

## Telemetry (opt-in `PEDRA_WRITE_PHASE_STATS=1`)

- `wal_lock_hold_ns` — time holding `wal.lock()`, distinct from acquire wait.
- `publish_cas_retries` — failed `published_seq` CAS.
- `LfStack::cas_failures` — join CAS misses (`cas_lf=`).
- Per-lane `try_write` miss count + wait-ns (`lane_c=`; Instant only on miss).

`ConcurrentDb::write_cycle_line` (the bench WRITEPHASE dump) calls
`write_cycle_forecast` and prints `cut=` / `qps_hat` / `off_wr_qps_hat` /
`lane_c` / `cas_lf`. Named test `rfc0192_write_cycle_line_uses_kernel`
drives real puts and asserts the dump is not `cut=lock_hold`.

## CLI

```
pedra scale-model write --leaders 4 --fixture linux-quiet
```

GET path (`--keys` / `--ram`) unchanged.

## Not this slice

- Linux quiet remeter of 0190 (P1.1) — guest unreachable; Darwin loadavg
  was 35 last look. Kernel names the cut when the slices exist; it does
  not invent a cartaz ratio.
- Skiplist TCB / write-off-lock — the kernel names; 0189/0190 execute.
