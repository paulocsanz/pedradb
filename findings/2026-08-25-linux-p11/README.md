# RFC-0062 P1.1 — Linux coluna B (G1 vs Rocks `sync=true`)

**VM:** `linux-gate-f208` (4 vCPU / 4 GB, brasil, Threadripper PRO 3975WX)
**Coluna:** B — Pedra JSON `sync=true` / `strongest-data-barrier-before-ok`
vs Rocks JSON `sync=true`. Rocks durability *label* says `host-default`
(harness string for `WriteOptions.sync=true` without `FULL_SYNC`); the
peer **did** set `wopts.set_sync(true)` (`fdatasync` class, `FULL_SYNC=0`).

Not the official cartaz (AGENTS.md peer is `sync=false`).

## p11a — uring `submit_and_wait` on `sync_data` — FAIL min 0.112

**When:** 2026-08-25T21:23:19Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11a`

| shape | min | vs 1.0 |
|---|---:|---|
| ycsb_c | 2.242 | PASS |
| ycsb_e | 4.734 | PASS |
| deps_lock_prewrite | 1.110 | PASS |
| deps_mvcc_latest | 3.147 | PASS |
| deps_apply_batch | 1.098 | PASS |
| deps_scan | 2.884 | PASS |
| kvrocks_get | 4.209 | PASS |
| kvrocks_scan | 39.174 | PASS |
| **ycsb_a** | **0.123** | FAIL |
| ycsb_b | 0.432 | FAIL |
| ycsb_d | 0.540 | FAIL |
| ycsb_f | 0.266 | FAIL |
| deps_cache_overwrite | 0.129 | FAIL |
| deps_raftlog | 0.420 | FAIL |
| kvrocks_set | 0.112 | FAIL |
| kvrocks_pipelined_set | 0.790 | FAIL |
| kvrocks_blob_set | 0.701 | FAIL |

Reads pass. 1-client writes lose. Darwin dirty same-class had ycsb_a **p50
empatado** (4.85 vs 4.73 ms `F_FULLFSYNC`) — Linux `fdatasync` is cheap, so
Pedra extra cost shows.

**Diagnosis:** production `IoUringFile::sync_data` used the ring
(`submit_and_wait` per Ok). Coluna A hid it (no fsync). Coluna B paid it
every put.

## p11b — POSIX `fdatasync` on WAL — FAIL min 0.153

**When:** 2026-08-25T21:42:02Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11b`
Production `sync_data` → `fdatasync_file`. Ring fsync stays **test-only**.

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_c | 2.916 | 3.069 | 3.069 / 2.916 / 3.313 | PASS |
| ycsb_e | 7.239 | 7.638 | 8.213 / 7.239 / 7.638 | PASS |
| deps_lock_prewrite | 1.199 | 1.310 | 1.339 / 1.199 / 1.310 | PASS |
| deps_mvcc_latest | 2.273 | 3.323 | 3.920 / 3.323 / 2.273 | PASS |
| deps_apply_batch | 1.097 | 1.121 | 1.273 / 1.097 / 1.121 | PASS |
| deps_scan | 2.572 | 2.891 | 3.293 / 2.891 / 2.572 | PASS |
| kvrocks_get | 4.307 | 4.753 | 5.115 / 4.307 / 4.753 | PASS |
| kvrocks_scan | 33.434 | 44.870 | 33.434 / 46.344 / 44.870 | PASS |
| kvrocks_pipelined_set | 1.289 | 1.407 | 1.407 / 1.289 / 1.604 | PASS |
| ycsb_a | 0.244 | 0.364 | 0.364 / 0.244 / 0.374 | FAIL |
| ycsb_b | 0.504 | 0.592 | 0.743 / 0.504 / 0.592 | FAIL |
| ycsb_d | 0.737 | 0.739 | 0.737 / 0.739 / 0.784 | FAIL |
| ycsb_f | 0.211 | 0.292 | 0.422 / 0.292 / 0.211 | FAIL |
| deps_cache_overwrite | 0.183 | 0.195 | 0.217 / 0.183 / 0.195 | FAIL |
| deps_raftlog | 0.393 | 0.439 | 0.464 / 0.393 / 0.439 | FAIL |
| kvrocks_set | **0.153** | 0.271 | 0.271 / **0.153** / 0.299 | FAIL |
| kvrocks_blob_set | 0.961 | 1.304 | 1.433 / 0.961 / 1.304 | FAIL |

`pipelined_set` left the fail list (1.29). ycsb_a 0.12 → 0.24. Still ~4–6×
behind on 1c writes. Do not round 0.153 up to a win.

**Diagnosis (named):** two stacked holes vs Rocks `PosixWritableFile::Allocate`:

1. `IoUringFile` did **not** override `EnvFile::preallocate` (trait default
   no-op). Production WAL `reserve_space` never reserved.
2. `pedradb_posix::preallocate_file` was Darwin-only (`F_PREALLOCATE`).
   Linux was `Ok(())`. G1 `fdatasync` of a growing WAL therefore paid
   **delayed allocation inside the Ok path**. Coluna A hid it (no
   fdatasync). Darwin `F_FULLFSYNC` ~5 ms hid it (p50 tied).

Also leftover: production `sync_all` / `sync_dir` still hit the ring
(vlog G1 used `sync_all`).

## p11c — Linux `fallocate(KEEP_SIZE)` + forward `preallocate` — image ready, VM down

**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11c`
(`sha256:db5b0d3b3d862d67537072d93530923d96bd39102693ac9eae566e998009cc08`,
linux/amd64). Local `docker run` sees the entrypoint + `fallocate` cut.

**Deploy 2026-08-26T15:07–16:08Z:** brasil não chegou a PID 1.

Não é FAIL do gate. Imagem p11c = 2.38 GiB (`p04a` + fonte). `linux-gate-f208`
está com `disk_mb=0` (P0.3 tinha 8 GiB). `scale --disk-mb` exige instância
running. `caixote service create` / `iac import` quebram no list-projects
(`map, expected a sequence`). Serviço novo `linux-gate-p11e` (`nginx:alpine`,
1 GiB disco) também `failed before first start` — o runtime de container
novo no brasil está fora, não só o tag p11c.

Cuts in-tree:

- Linux `fallocate(FALLOC_FL_KEEP_SIZE)` from current `i_size` (logical
  size unchanged — recovery never sees reserved zeros).
- `IoUringFile::preallocate` forwards to posix.
- Production `sync_all` / `sync_dir` POSIX (ring test-only, same as
  `sync_data`).
- `sync_data_strong` = `File::sync_data` (Darwin `F_FULLFSYNC`, Linux
  `fdatasync`).
- vlog `sync_pending` uses `sync_data_strong` (same class as WAL G1),
  not `sync_all`.

Gate unchanged: 3 quiet rounds, `min_ratio > 1.0`, both JSON `sync=true`.

## p11d — skip empty-vlog fsync (in-tree; VM failed before PID 1)

**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11d`
(`sha256:da70a9ab8e08785118630ac7bed3bcc7019901dc928bbaf251c4cb0fd6e50082`).

**Named leftover after posix WAL `fdatasync` + fallocate:** the parity
harness enables blob (`min_blob_size=4096`). Open creates `VALUES.vlog`.
Every G1 commit called `vlog_prepare_wal(true)` → `sync_pending` →
`sync_data_strong` **even when the put never spilled** (ycsb_a is 100 B).
That is a second barrier per Ok vs Rocks (blob off by default).

In-tree:

- `ValueLog::sync_pending` no-ops when `pending` is empty and nothing has
  been `write()`n since the last barrier. A real spill still pays exactly
  one strong class barrier before Ok.
- Tests: `sync_pending_skips_empty_and_barriers_once_when_dirty`,
  `g1_small_put_does_not_fsync_empty_vlog`.
- Compact worker skips fold/stage while `commit_inflight > 0` (G1
  `lone_commit` drops the write lock for `fdatasync`; 1c still counts as
  `writes_active()==1`).

## p11e — remesura Linux coluna B — FAIL min 0.562 **só** `deps_raftlog`

**When:** BUILD_OK 16:43:00Z, gate 16:43:51Z
**VM:** `linux-gate-f208` running `ghcr.io/paulocsanz/pedradb-linux-gate:p11e`
(`/data` ext4 on `vdb`; overlay tmpfs ~1 GiB). `P11_START` 16:36:49Z.
**Coluna B:** Pedra `sync=true`, Rocks `sync=true` (`FULL_SYNC=0`). 3 rounds.

fallocate + skip-empty-vlog **fecharam as writes 1c** que no p11b estavam 0.15.

| shape | min | mediana | vs 1.0 |
|---|---:|---:|---|
| ycsb_a | 1.595 | 1.624 | PASS |
| ycsb_b | 1.874 | 2.655 | PASS |
| ycsb_c | 2.732 | 3.137 | PASS |
| ycsb_d | 2.448 | 2.503 | PASS |
| ycsb_e | 11.851 | 12.824 | PASS |
| ycsb_f | 1.435 | 1.548 | PASS |
| deps_cache_overwrite | 1.373 | 1.962 | PASS |
| deps_lock_prewrite | 1.412 | 1.431 | PASS |
| deps_mvcc_latest | 3.647 | 4.043 | PASS |
| deps_apply_batch | 1.380 | 1.410 | PASS |
| deps_scan | 2.664 | 3.013 | PASS |
| kvrocks_get | 4.665 | 4.984 | PASS |
| kvrocks_set | 1.285 | 1.299 | PASS |
| kvrocks_scan | 32.315 | 35.568 | PASS |
| kvrocks_pipelined_set | 2.721 | 2.999 | PASS |
| kvrocks_blob_set | 2.032 | 2.096 | PASS |
| **deps_raftlog** | **0.562** | **0.745** | FAIL 0.837 / 0.745 / **0.562** |

`RESULT=P11_FAIL min_ratio=0.5616201632492395 fail=deps_raftlog`

16/17 oficiais >1.0. p11b min 0.153 com 8 write FAILs → um shape.
Empty-vlog fsync era o 4× das writes 1c. **Não** arredondar 0.562.

O que resta é o mesmo shape do P0.4 na coluna A: 16 appends + get
`idx-1` no CF `raftlog`. Não relitigar skiplist. ycsb_a 1-put já é 1.59×
— o fd não é o teto deste batch.

Boot: p04a `ENTRYPOINT` (script em `p04_entrypoint.sh`). p11c/p11d
mudaram ENTRYPOINT e morreram em `Starting VM`. `caixote push`
source-builder está partido (CHV TAP / RFC 0192).

Digest: `sha256:cb2aeccddb0fb0ed21684b9273178e86de13a619a272d84bef8babb1f3a959e7`.
Official cartaz remains coluna A.

## p11f — intern consecutive equal payloads — FAIL min 0.441

**When:** 2026-08-26T17:05:24Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11f`
(`sha256:8255c4effec27c78be7ee7a4461bfb6f58d2e866a90a041047291353f6c328f9`)
**Cut:** `share_consecutive_equal_values` in `prepare_write_ops` +
`write_cf_owned` shares `Bytes` when the next payload equals the last.
WAL v2 omits 15×100 B on raftlog. Tests:
`share_consecutive_equal_values_enables_v2`,
`write_cf_owned_sixteen_same_payload_roundtrip`.

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_a | 1.149 | 1.445 | 1.445 / 1.599 / 1.149 | PASS |
| ycsb_b | **0.441** | 1.170 | 1.17 / 1.978 / **0.441** | FAIL (1 round) |
| ycsb_d | **0.464** | 2.291 | 2.291 / 2.719 / **0.464** | FAIL (1 round) |
| **deps_raftlog** | **0.939** | 0.964 | 0.939 / 0.978 / 0.964 | FAIL (all 3) |
| kvrocks_set | 1.341 | 1.396 | | PASS |
| kvrocks_blob_set | 2.120 | 2.202 | | PASS |

`RESULT=P11_FAIL min_ratio=0.441 fail=ycsb_b,ycsb_d,deps_raftlog`

raftlog **0.562 → 0.939** (stable across 3). ycsb_b/d mins are **one**
round each (median still >1); p11e ycsb_b already had a 9.41 outlier.
Do not revert intern. Do not round 0.94 to a win.

Official cartaz remains coluna A.

## p11g — LAST_PROBE 8→16 (idx-1 TLS) — FAIL min 0.971 só raftlog

**When:** `P11_START` 18:45:58Z, `BUILD_OK` 18:52:05Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11g`
(`sha256:a8dd5cbc0e8a7eaf32727195351fa49110f8a5486ca3d064c8138733d4d815d3`)
**Cut:** `LAST_PROBE` 8→16 so the 16-key raftlog batch keeps `idx-1` in
LAST_CF. Test `write_cf_owned_warms_named_get_tls` asserts TLS-hot for
all 16 keys and **one** WAL barrier. p04a ENTRYPOINT kept.

| shape | min | vs 1.0 |
|---|---:|---|
| ycsb_a | 1.249 | PASS |
| ycsb_b | 1.640 | PASS (p11f 0.441 was 1 dirty round) |
| ycsb_d | 1.945 | PASS |
| kvrocks_set | 1.401 | PASS |
| **deps_raftlog** | **0.971** | FAIL 0.971 / 0.994 / **1.111** |

`RESULT=P11_FAIL min_ratio=0.970724016349234 fail=deps_raftlog`

16/17. raftlog 0.939→0.971; r3 already >1. Do not round 0.971 up.
Intern stays. ycsb_b/d dirty rounds of p11f did not repeat.

p11g already had `lone_commit` → `group_finish` in-lock (drop+reacquire
was gone). Remaining 3% is not that dance.

## p11h — ring+hash (both) + one WAL lock — FAIL min 0.843, dirty

**When:** BUILD 23:16:53Z, gate 23:23:46Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11h`
(`sha256:9fb2b5b7e046d7a0296c687fe0352d44723fcaa3bd95ad48540d4a053c005c03`)

| shape | min | rounds | vs 1.0 |
|---|---:|---|---|
| ycsb_a | 1.642 | 1.642 / 1.661 / 1.666 | PASS |
| ycsb_c | 3.115 | **21.164** / 5.834 / 3.115 | PASS (r1 dirty) |
| **deps_raftlog** | **0.843** | **2.44** / **0.843** / **0.945** | FAIL |

`RESULT=P11_FAIL min_ratio=0.8432914229608409 fail=deps_raftlog`

r3 `load1=4.56`. ycsb_c r1=21× is Rocks stall, not a Pedra win.
Ring **and** 16-probe hash together is extra write tax vs p11g. Not
proven. Reverted the hash write-through on LAST_CF; WAL one-lock stays.

## p11i — LAST_CF ring-only + inter-round quiet — FAIL min 0.991 só raftlog

**When:** BUILD_OK 23:46:11Z, gate 23:51:06Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11i`
(`sha256:6fd4f10a60e0249458f8738670f8110823bc836f434e621d90924c78005b1794`)
**VM:** first deploy died before PID 1 (`disk_mb=0`). Recovered nginx:alpine
+ `add-volume` `/data` 8 GiB elastic, then p11i.

**Cuts vs p11g:** LAST_CF `store` is the 16-slot ring only (no hash walk on
the 16-put path). Get checks ring then 8-probe. WAL one-lock write+fd
(from p11h). `wait_quiet` load1<1.5 ×3 before isolated and each suite
round. p11h ring+hash reverted.

First deploy of this tag: `all containers failed before first start`.
Not a gate FAIL.

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_a | 1.517 | 1.526 | 1.517 / 1.625 / 1.526 | PASS |
| ycsb_b | 1.356 | 1.990 | 1.356 / 2.126 / 1.99 | PASS |
| ycsb_c | 2.430 | 2.745 | 2.430 / 3.358 / 2.745 | PASS |
| ycsb_d | 1.366 | 2.294 | 1.366 / 2.352 / 2.294 | PASS |
| ycsb_e | 10.788 | 13.023 | | PASS |
| ycsb_f | 1.378 | 1.613 | | PASS |
| deps_cache_overwrite | 1.468 | 1.596 | | PASS |
| deps_lock_prewrite | 1.742 | 2.067 | | PASS |
| deps_mvcc_latest | 2.866 | 3.313 | | PASS |
| deps_apply_batch | 1.697 | 1.913 | | PASS |
| **deps_raftlog** | **0.991** | **1.039** | 1.039 / **0.991** / 1.29 | FAIL |
| deps_scan | 2.724 | 2.953 | | PASS |
| kvrocks_get | 4.257 | 4.852 | | PASS |
| kvrocks_set | 1.273 | 1.330 | | PASS |
| kvrocks_scan | 42.436 | 45.055 | | PASS |
| kvrocks_pipelined_set | 2.968 | 3.073 | | PASS |
| kvrocks_blob_set | 1.971 | 2.324 | | PASS |

`RESULT=P11_FAIL min_ratio=0.9911528624905533 fail=deps_raftlog`

16/17. Quiet worked (r1 rocks-kvr `load1=0.19`; no ycsb_c 21×). Best
coluna B raftlog min so far (p11g 0.971, p11h 0.843 dirty). **Do not
round 0.991.** Median 1.039, r3 1.29. Isolated empty-DB raftlog
`RESULT=P11_ISO_deps_raftlog_FAIL min=0.935` — the leftover is the
16-append itself, not apply_batch mem fat.

Official cartaz remains coluna A.

## p11j — lone_sync_commit one WAL lock encode+write+fd — PASS min 1.013

**When:** BUILD 01:46:57Z, gate 01:57:15Z
**Image:** `ghcr.io/paulocsanz/pedradb-linux-gate:p11j`
(`sha256:ce2ea247ad3e702b34ba7bf87dea21019f91a4c1abe4a51a4585038b9f822d67`)
**VM:** `linux-gate-f208` (4 vCPU / 4 GB, brasil, Threadripper PRO 3975WX)
**Coluna:** B — both `sync=true`, `FULL_SYNC=0`. r3 rocks-kvr `load1=0.10`.

**Cut:** 1c G1 skips `GroupInFlight`. `lone_sync_commit` prepares, then
**one** `wal.lock()` for encode + `write` + `fdatasync`, then apply.
p11i encoded in `group_start` and write+fd in `group_finish` (two mutex
hops + Vec). Isolated 0.935 was that leftover.

| shape | min | mediana | rounds | vs 1.0 |
|---|---:|---:|---|---|
| ycsb_a | 1.572 | 1.640 | 1.572 / 1.658 / 1.64 | PASS |
| ycsb_b | 1.323 | 1.997 | 1.323 / 2.041 / 1.997 | PASS |
| ycsb_c | 2.593 | 2.916 | 2.593 / 3.278 / 2.916 | PASS |
| ycsb_d | 1.640 | 2.353 | 1.64 / 2.353 / 2.403 | PASS |
| ycsb_e | 8.630 | 10.567 | 10.567 / 8.63 / 12.435 | PASS |
| ycsb_f | 1.057 | 1.416 | 1.416 / 1.057 / 1.623 | PASS |
| deps_cache_overwrite | 1.571 | 1.794 | 1.841 / 1.794 / 1.571 | PASS |
| deps_lock_prewrite | 1.829 | 1.972 | 1.972 / 2.193 / 1.829 | PASS |
| deps_mvcc_latest | 2.631 | 2.698 | 2.631 / 2.698 / 3.854 | PASS |
| deps_apply_batch | 1.700 | 1.755 | 2.015 / 1.7 / 1.755 | PASS |
| **deps_raftlog** | **1.013** | **1.019** | 1.019 / **1.013** / 1.021 | **PASS** |
| deps_scan | 2.873 | 2.907 | 2.907 / 2.873 / 8.402 | PASS |
| kvrocks_get | 4.601 | 4.645 | 4.645 / 4.601 / 4.712 | PASS |
| kvrocks_set | 1.458 | 1.515 | 1.652 / 1.515 / 1.458 | PASS |
| kvrocks_scan | 31.879 | 33.085 | 46.789 / 33.085 / 31.879 | PASS |
| kvrocks_pipelined_set | 3.013 | 3.057 | 3.671 / 3.057 / 3.013 | PASS |
| kvrocks_blob_set | 1.712 | 2.115 | 2.15 / 2.115 / 1.712 | PASS |

`RESULT=P11_PASS min_ratio=1.013`

**17/17 min > 1.0.** raftlog 3/3 > 1 (1.013–1.021), not a lucky round.
Piso é raftlog 1.013 e ycsb_f 1.057. Não é 2×; cartaz oficial continua
coluna A (`P04_PASS` 1.014). ISO empty-DB lines rolled off the serial
buffer (gate JSON is suite-only).

p04a ENTRYPOINT overlay + `wait_quiet` kept. Tests:
`lone_g1_sixteen_same_payload_one_barrier`,
`write_cf_owned_warms_named_get_tls`.
