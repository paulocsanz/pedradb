# RFC-0176 P2.2 — regime ladder on linux-gate-p149b

**Date:** 2026-09-07T04:20Z  
**Guest:** `linux-gate-p149b` / `cnt_4a95c57660154193a754143e72b019f1`  
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p149k`  
**Box:** 4 vCPU, 4 GiB (`MemTotal` 3 985 628 kB = 4 081 283 072 B)  
**Not** Darwin. **Not** a laptop DIAG.

## Exec

tcp/22 refused; serial is append-only. QEMU Guest Agent **does** listen:
vsock unix `CONNECT 1024` → qemu-ga 10.0.0 (`guest-exec`). No rootfs inject;
`p12_warm10.sh` left sleeping.

`scale_forecast` (same kernel as `pedra scale-model`) was rustc'd on the
guest from `scale_kernel.rs`. No second formula. No new engine probe.

## Per-cell `mode=` (kernel on this box)

| n | ram | S | cap | `mode=` |
|---|---:|---:|---:|---|
| 10M | 4 GiB nominal | 2.45 GiB | 3.00 GiB | **hot** |
| 10M | MemTotal | 2.45 GiB | 2.80 GiB | **hot** |
| 25M | 4 GiB nominal | 5.71 GiB | 3.00 GiB | **bounded-cache** |
| 25M | MemTotal | 5.71 GiB | 2.80 GiB | **bounded-cache** |

Raw: [`mode-10m.txt`](mode-10m.txt), [`mode-25m.txt`](mode-25m.txt).

## Measured T (corroborates the regime, not a retune)

| n | disk after settle | get_hit p50 | probes/op | class |
|---|---:|---:|---:|---|
| 10M | 2.42 GiB | **6.3 µs** | 1.00 | RAM (WARM10 serial 2026-09-06T22:35Z, w=1) |
| 25M | 6.04 GiB | **31.3 µs** | 1.00 | disk (`PEDRA_SETTLE_WARM=0`; this image has no skip-over-cap) |

25M cell: [`25m-cell.log`](25m-cell.log). Hydrate 25M in 58.1 s (0.43 M/s),
254 B/e. `rc=0`. Store freed after.

25M predict happy = 48.5 µs; measured 31.3 µs is inside `[best/4, worst×4]`
(1.1–567 µs). τ **not** retuned. Point path stays 1 SST/op — the jump is
\(H\), not \(P\).

10M predict best = 3.3 µs; measured 6.3 µs (WARM residual / noisy). Same
band. Do not quote 10M as a 1B claim.

## What this is not

- Not vs Rocks. Peer remains `sync=false`.
- Not a 1B/10B load.
- Not engine `ram_line` (this image's `snapshot_backends` does not print
  `mode=`; the kernel printer on the box does).
- Not G1 write-shape wins.
