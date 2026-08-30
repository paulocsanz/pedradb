# RFC-0149 P2.1 — Linux coluna A maioria >3×

**When:** 2026-08-29T08:49Z–08:58Z  
**Peer:** Pedra `PEDRA_PARITY_ASYNC=1` vs Rocks `WriteOptions.sync=false`.  
**Pass:** ≥9/17 median > 3.0.  
**Host:** AMD Ryzen Threadripper PRO 3975WX, brasil, `platform-4d79d23b…`.

## Control plane / VM (blocked)

Portaria 200. Provision field is `cpus` (not `vcpus`). `linux-gate-p149b`
was 4 vCPU / 8192 MB disk. VM error:

```
Failed to route 10.0.0.112/32 via 169.254.1.2: Nexthop has invalid gateway
```

Same on public `nginx:alpine`. `veth_blocks` still 169.254.1.1/1.2; netns
is 169.254.184.218/30. 533 instances `destroying`. Not GHCR.

## What actually ran

Not QEMU 4 vCPU. systemd `CPUQuota=400%` + `CPUAffinity=0-3`, chroot of
p11j `oci.tar` + current crates (no p04a prebuilt). Same harness as Mac
P0 (`ROCKS_YCSB_OPS=2000`).

| run | disk | majority median>3 | min |
|---|---|---:|---:|
| tmpfs `/tmp` | RAM | **6/17 FAIL** | 1.068 |
| NVMe bind `/tmp` | `/var/lib/caixote-nvme` | **7/17 FAIL** | 1.058 |

JSON: `tmpfs-cgroup4/`, `nvme-cgroup4/`.

### NVMe 4-CPU table (the number to quote if quoting this box)

| shape | min | median | >3 |
|---|---:|---:|:---:|
| ycsb_a | 2.529 | 2.641 | |
| ycsb_b | 2.281 | 2.519 | |
| ycsb_c | 2.714 | 3.022 | yes |
| ycsb_d | 2.285 | 2.634 | |
| ycsb_e | 11.477 | 12.396 | yes |
| ycsb_f | 2.070 | 2.246 | |
| deps_cache_overwrite | 2.564 | 2.889 | |
| deps_lock_prewrite | 1.824 | 1.882 | |
| deps_mvcc_latest | 3.091 | 3.444 | yes |
| deps_apply_batch | 1.630 | 1.641 | |
| deps_raftlog | 1.058 | 1.098 | |
| deps_scan | 3.297 | 3.441 | yes |
| kvrocks_get | 4.672 | 4.722 | yes |
| kvrocks_set | 2.814 | 2.974 | |
| kvrocks_scan | 35.576 | 38.600 | yes |
| kvrocks_pipelined_set | 3.070 | 3.173 | yes |
| kvrocks_blob_set | 2.062 | 2.072 | |

Mac P0 was **12/17**. Rocks on Linux is faster; ratios fall. This is
**not** “we beat Rocks” (async vs `sync=false`, same class). Official
Linux virt gate still blocked on TAP.

`RESULT=P149_FAIL over_med=7/17 min_ratio=1.058`

## CHV 4 vCPU (official virt shape)

Host default `CAIXOTE_VMM_RUNTIME=cloud-hypervisor`. Guest 4 CPUs,
~4 GiB, kernel `6.12.94-0-virt`. Image `p149a`, cargo rebuild in-guest
(not p04a prebuilt). Coluna A, `ops=2000`.

| run | note | majority | min |
|---|---|---:|---:|
| 17:12Z | extra 1 vCPU workers on host | **8/17 FAIL** | 0.761 |
| 17:26Z | quiet; ACPI poweroff mid r3 | no RESULT | — |
| **20:01Z** | **quiet, 3 rounds** | **4/17 FAIL** | **1.038** |

Quiet table (the Linux virt number):

| shape | min | median | >3 |
|---|---:|---:|:---:|
| ycsb_a | 2.068 | 2.234 | |
| ycsb_b | 1.995 | 2.067 | |
| ycsb_c | 2.573 | 2.595 | |
| ycsb_d | 2.337 | 2.400 | |
| ycsb_e | 12.043 | 12.073 | yes |
| ycsb_f | 1.869 | 2.070 | |
| deps_cache_overwrite | 1.129 | 2.556 | |
| deps_lock_prewrite | 2.025 | 2.032 | |
| deps_mvcc_latest | 2.419 | 2.442 | |
| deps_apply_batch | 1.555 | 1.722 | |
| deps_raftlog | 1.038 | 1.058 | |
| deps_scan | 2.436 | 2.625 | |
| kvrocks_get | 4.921 | 5.005 | yes |
| kvrocks_set | 2.495 | 2.523 | |
| kvrocks_scan | 23.628 | 43.280 | yes |
| kvrocks_pipelined_set | 3.346 | 3.484 | yes |
| kvrocks_blob_set | 2.339 | 2.403 | |

`RESULT=P149_FAIL over_med=4/17 min_ratio=1.038`

Mac P0 12/17 does not count. CHV 4 vCPU is **4/17**. Not a Rocks win
(async vs `sync=false`).
