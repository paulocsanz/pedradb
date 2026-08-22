# Adversarial audit — WAL persistency + performance claims (2026-08-21)

Hypothesis under attack (from the user): *"RocksDB is very efficient, so we must have a WAL
persistency bug; and we can't be 5× faster in most things and ≥95% as fast in everything."*

Scope: `pedradb-core` write/commit path, WAL writer/reader/recovery, vlog (WiscKey large
values), MANIFEST/CURRENT publish, GC (vlog/blob/compaction), read boundaries. Peer for all
performance claims: **RocksDB default** (`WriteOptions.sync=false`, `ROCKS_PARITY_SYNC=0`),
per AGENTS.md / RFC-0041. Pedra `fdatasync`s before `Ok` — that asymmetry is the product.

---

## 1. Verdict on the friend's hypothesis

**The core WAL persistency path is correct**, and Pedra's speed is real where it exists —
but not because of skipped fsyncs (§4) — **and the performance claim as stated is false**:
on fresh same-box runs the async column reaches 5× on 2/17 official shapes (12%) with a
0.79× minimum, and the g1 product column clears the RFC-0041 2× floor on 4/17 shapes on
this hardware (§5). The real durability bugs the audit found are one layer out — in
value-log GC and at the read boundary (§3): two are HIGH with deterministic repros that
lose **sync-acked** writes after a crash (F2, F3).

Positive verification of the core path (all at the Env seam, not internal counters): every
`Ok` on a sync write is preceded by a real libc `fdatasync` covering the appended bytes; a
lone client cannot share or skip fsyncs (1.00 real fdatasyncs/op, measured); group commit
amortizes but never skips (≤1 per client); the async column performs zero fsyncs until an
explicit sync; injected sync failure fences the DB fail-closed; torn tails are truncated
before new appends; PointInTime recovery never serves a silently-empty prefix.

## 2. Method

- Collector sweep of the storage layer (`/tmp/audit-collect.txt`, 36,891 lines) + manual
  read of wal/{writer,reader,format,crc}, db commit/rotate/flush/GC paths, concurrent.rs
  write group, manifest, vlog, occ, posix env, recovery.
- New adversarial suite: `crates/pedradb-core/tests/wal_durability_adversarial.rs`
  (12 tests, all green after the fixes: 9 core-contract proofs, the F2 + F3 regression
  tests — formerly `#[ignore]`d failing proofs — and the F1 corruption regression).
- New bench: `crates/pedradb-core/benches/fsync_amortization.rs` (Env-seam syscall counts).
- Parity campaign: `scripts/audit_parity_20260821.sh` — two columns (G1 product, async),
  vs real RocksDB default peer, default suite = 17 official shapes (§5).
- Existing workspace suite re-run: 1094 passed / 5 failed; all 5 failures reproduce green in
  isolation and are pre-existing test-hygiene issues in user-WIP files (dcs unsalted
  `/tmp/pedradb-dcs-cmd-*` dirs + stale `LOCK`; raft pid-salted port collisions), not engine
  bugs.

## 3. Findings

| ID | Sev | Finding | Evidence |
|----|-----|---------|----------|
| F2 | **HIGH (proven)** — **FIXED 2026-08-21** | vlog/blob GC deletes pre-GC value-log sources on an unverified "flush emptied the WAL" premise; stale pre-GC vlog pointers in the un-rotated WAL shadow the remapped SST after crash replay; the dangling read is swallowed to a silent miss. Loses a sync-acked write. | db.rs:3516 `try_rotate_wal` silently skips when `commit_inflight>0`/mem non-empty; db.rs:3115 flush returns Ok regardless; db.rs:3876 comment states the premise; db.rs:4040 `compact_blob` same premise + `remove_file`. Repro: `vlog_gc_during_offlock_fsync_window_*` (was `#[ignore]`d; now a green regression test). |
| F1 | **HIGH** — **FIXED 2026-08-21** | Value-log CRC errors are fail-closed internally but **swallowed at public read boundaries**: corrupt vlog payload reads as `None` (looks deleted), scans silently skip the entry, change-feed returns the raw 9-byte VLG pointer as the user value. | Fixed at every boundary: `get_at`/`multi_get_at`/`changes` propagate `Err(CorruptValue)`; `get`/`multi_get`/scan streams/`changes_after` fail-stop. New `CoreError::CorruptValue` → compat `ErrorKind::Corruption`. Regression: `corrupt_vlog_payload_is_loud_not_missing` (green). |
| F3 | **HIGH (proven)** — **FIXED 2026-08-21** | A GC round on an un-promoted state (`vlog_use_new=true`, `.new` staged — the crash-after-install state) deletes the live `.new` and rebuilds it; the failure path leaves a **truncated** `.new` on disk. With ≥ MAGIC bytes landed, recovery trusts it and SST pointers past the truncation dangle → acked large values silently read as missing. With 0 bytes landed, `Db::open` fail-closes with `vlog too short` — the DB cannot reopen without manual file surgery. | Fixed: `compact_vlog_stage_manifest` promotes the staged round before rewriting; `rewrite_live_to_new` never removes the staging path first. Repro: `second_gc_on_unpromoted_state_*` (was `#[ignore]`d; now a green regression test). |
| F4 | MEDIUM — **FIXED 2026-08-21** | Dir-sync of first-create dirents for the value log is best-effort (`let _ =`): the payload is `sync_all`ed but if the swallowed `sync_dir` fails/doesn't happen, a crash can lose the *filename* while the WAL pointer to it is durable — an acked large-value write vanishes. | All five sites now propagate: `open_blob`, `open_with_flag` create branch, `rewrite_live_to_new`, `rewrite_live_to_blob`, `promote_new_and_reopen`. |
| F5 | MEDIUM — **FIXED 2026-08-21 (in-repo)** | `Env` default methods bypass the Env seam: `is_dir` shells out to `fs::metadata` directly and maps errors to `Ok(false)`. Any non-StdEnv (test/remote/io_uring) gets core semantics decided by the real filesystem, silently. | Default now only maps `NotFound`→`Ok(false)` (other errors propagate) + override contract documented; `RecordingEnv` (in-memory — the real bug) decides from its image; `FailingEnv`/`FailingEnvArc` delegate through the seam. Two test envs inside the user's live `history.rs` WIP still use the default (file not editable this session). |
| F6 | LOW (in-spec) | Residual power-loss window for `sync=true`: stale CURRENT + already-truncated WAL + orphan-SST GC can drop flushed-but-unpublished data; requires a `sync_dir` lie. Documented product boundary, not a code defect. | design docs / RFC-0041 discussion. |

**F2 fix (applied)**: `Db::ensure_wal_rotated_for_gc` — after the flush that precedes
`compact_vlog_stage_manifest`/`compact_blob`, the round is refused (`Err`, retryable)
unless the WAL rotation actually happened. The predicate is the exact skip-condition of
`try_rotate_wal` (`commit_inflight == 0` and mem/imm/pin/parked empty) evaluated after a
completed flush under the write lock — no new records can be appended while it is held,
so it is equivalent to "the WAL was rotated". Single-writer flows never trip it; under
concurrent load the round returns a clear error and auto-GC logs and defers.

**F1 fix (applied)**: new `CoreError::CorruptValue` for vlog len/CRC mismatches;
`get_at`/`multi_get_at`/`changes` propagate it; Option/iterator-shaped APIs (`get`,
`multi_get`, scan streams, `changes_after`) fail-stop with an actionable message instead
of returning a lying miss / empty value / raw pointer. The CHANGELOG cache rebuild and
the history-archive chunk keep best-effort semantics (cache staleness / pointer
preservation, never invented bytes).

**F3 fix (applied)**: `compact_vlog_stage_manifest` promotes any staged-but-unpromoted
round before rewriting, so a rewrite only ever replaces a non-live staging file;
`rewrite_live_to_new` no longer removes the staging path before writing (`env.create`
truncates in place — the same contract `Wal::create_on` relies on). A mid-rewrite
failure now degrades to a plain retryable `Err` with the live layout intact.

### Verified-correct (attacked, held)

- Commit ordering: WAL append → `fdatasync` → mem apply → `Ok` (single-writer); group
  leader: apply under write lock → drop lock → off-lock fsync → `publish_sequence`
  (Release/Acquire). Reads see only published (post-fsync) sequence.
- `begin_commit`/`end_commit` correctly block WAL rotation during the off-lock window (the
  *skip* is safe for rotation's own purpose — the bug is only GC relying on the flush).
- Recovery: torn tail truncated to `last_good` + `sync_data` before new appends; CRC/orphan
  fragments fail-stop with 3-event escalation journal; PointInTime serves a reported prefix.
- MANIFEST/CURRENT: tmp → fsync → rename → dir-fsync; WAL rotate pays SST+MANIFEST
  durability first.
- vlog payloads `sync_all` (stronger class) before their WAL pointers become durable.
- OCC discards `WriteOptions` only in the always-sync direction.

## 4. fsync amortization bench (new)

`cargo bench -p pedradb-core --bench fsync_amortization` — Env-seam counts of **real** WAL
`fdatasync` syscalls:

| shape | ops/s | real WAL fdatasyncs | amortization (ops/fsync) |
|-------|-------|--------------------|--------------------------|
| lone_sync_1c (2000 ops) | 138 | 2000 | **1.00** |
| group_sync_2c | 268 | 1079 | 1.85 |
| group_sync_4c | 319 | 831 | 2.41 |
| group_sync_8c | 774 | 443 | 4.51 |
| lone_async_1c | 459,510 | 1 (final explicit sync) | 2000.00 |

Lone-client amortization of 1.00 is the physics ceiling (RFC-0041 P1.2): if this ever exceeds
~1 without group commit, speed is being bought with durability. Group commit amortizes ≤ 1
fsync/client as designed. The async column is honestly async.

## 5. Parity vs RocksDB default (claim: 5× most / ≥95% everything)

Campaign: `scripts/audit_parity_20260821.sh` — `ROCKS_PARITY_SYNC=0` (default peer,
`rocks-parity-compare` exits 2 on a `sync:true` peer), default suite (ycsb,deps — the 16
official shapes), records=1024, ops=30000, payload=1000B, zipfian, 4 clients. Columns:
**g1** (Pedra fdatasync-before-Ok, product column, floor ≥2× per RFC-0041) and **async**
(`PEDRA_PARITY_ASYNC=1`, the RFC-0044 5×-class column under audit).

Environment note: shared box, ambient load ~50 from other sessions during the runs; db
directories are removed after each phase. An initial all-suite (44-shape) attempt was
killed after the compat WAL (`CURRENT.log`) grew to **97 GB** in ~30 min (~10× the logical
write volume). Unverified at scale; plausible mechanism is F2's sibling — WAL rotation is
best-effort (`try_rotate_wal` skips while `commit_inflight>0`/mem non-empty, which under
4-client load is near-always) so the WAL may never rotate during a campaign. Recorded as
open item **O1**: measure WAL rotation frequency under sustained concurrent write load.

Fresh same-box run (both engines, minutes apart, `ROCKS_PARITY_SYNC=0` — the official
default peer; no sync peer was run). Shared box, ambient load 16-50 from other sessions.

**g1 — product column (Pedra fdatasync-before-Ok vs RocksDB default):**

| shape | ratio | | shape | ratio |
|---|---|---|---|---|
| deps_scan | 5.87 | | ycsb_d | 0.30 |
| ycsb_e | 4.59 | | deps_raftlog_mc4 | 0.28 |
| ycsb_c | 4.53 | | ycsb_a_mc4 | 0.20 |
| deps_mvcc_latest | 2.81 | | deps_raftlog | 0.10 |
| deps_apply_batch_mc4 | 0.75 | | ycsb_f | 0.08 |
| ycsb_f_mc4 | 0.52 | | deps_cache_overwrite | 0.08 |
| deps_apply_batch | 0.47 | | ycsb_b | 0.07 |
| deps_lock_prewrite | 0.45 | | ycsb_a | 0.04 |
| deps_cache_overwrite_mc4 | 0.40 | | | |

≥2× on **4/17** shapes (24%), median 0.40×, min 0.041× (ycsb_a). Only read-mostly shapes
clear the RFC-0041 floor here; every write-heavy shape pays Pedra's real per-write
`fdatasync` against Rocks' async WAL. Environment caveat: this box's `fdatasync` latency
(APFS + ambient load) is far worse than the hardware the RFC-0041 floor was set on, so the
absolute g1 ratios are not comparable to the ledger's — but they are what this machine
delivers today, and they bracket the physics: sync-class writes cannot match async writes
per-op on any disk.

**async — RFC-0044 5×-class column (Pedra async vs RocksDB default), claim under audit:**

| shape | ratio | | shape | ratio |
|---|---|---|---|---|
| ycsb_e | 58.4 | | ycsb_a | 2.64 |
| deps_cache_overwrite_mc4 | 7.19 | | deps_mvcc_latest | 2.54 |
| ycsb_a_mc4 | 4.95 | | deps_lock_prewrite | 2.12 |
| ycsb_b | 4.93 | | deps_apply_batch | 1.97 |
| ycsb_c | 4.89 | | ycsb_f | 1.77 |
| deps_scan | 4.73 | | deps_apply_batch_mc4 | 1.73 |
| deps_cache_overwrite | 3.71 | | deps_raftlog_mc4 | 1.68 |
| ycsb_d | 3.38 | | **deps_raftlog** | **0.79** |
| ycsb_f_mc4 | 2.87 | | | |

**Claim verdict ("5× faster in most things, ≥95% as fast in everything"): REFUTED on both
halves.** ≥5× on **2/17** shapes (12% — not "most"); median 2.87×. Below 0.95× on
**1/17** (`deps_raftlog` 0.79× — a synced-shape where Rocks' async writes dominate).
This matches the repo's own honest RFC-0044 ledger (which already recorded
`deps_lock_prewrite` 0.94×, F 1.66×, blob ~2×). Phase-order bias note: in each column the
Pedra side ran before the Rocks side while ambient load was falling, which understates
Pedra slightly — it cannot flip 12% into a majority.

## 6. Artifacts

- Tests: `crates/pedradb-core/tests/wal_durability_adversarial.rs` (12, all green after
  the fixes: 9 core-contract proofs, F2 + F3 regression tests — formerly `#[ignore]`d
  proofs — and the F1 corruption regression).
- Bench: `crates/pedradb-core/benches/fsync_amortization.rs` + Cargo.toml registration.
- Script: `scripts/audit_parity_20260821.sh`.
- Compile fix to user WIP: `rocksdb-compat/src/lib.rs` — map `CoreError::CorruptHistory` →
  `ErrorKind::Corruption` (E0004 non-exhaustive match introduced by uncommitted history.rs
  work); the F1 fix added `CoreError::CorruptValue` → `ErrorKind::Corruption` there too.
- Outputs: `findings/2026-08-21-adversarial-audit/{fsync_amortization.txt, parity.log,
  g1/, async/}`.
