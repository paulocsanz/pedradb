Hydrate phase profile — local 6M, v23-equivalent tree + WRITEPHASE print (2026-09-01)
=================================================================================

Instrumentation: `PEDRA_WRITE_PHASE_STATS=1` now prints one `WRITEPHASE`
summary line at `Db` teardown (db.rs Drop — same env-gating family as
FLUSH_DIAG). Numbers below are the real vendored bench (`--bench zznone`,
6M entries, 256 MiB cache, bulk on), two runs.

WRITEPHASE (5860 commits = the bench's 1024-op apply batches):

    commits=5860 prepare_ms=  79.9 wal_ms=22529.3 mem_ms=1130.7 publish_ms=0.8
    commits=5860 prepare_ms= 102.3 wal_ms=23282.7 mem_ms=1444.2 publish_ms=0.8
    (flush_check/lock_wait ≈ 0 in both)

Hydrate wall 27.2 / 28.6 s. So locally the commit path is 81–84 % WAL —
but `sample(1)` during the pedra window splits that WAL time:

- **66 % of ALL samples: `pedradb_posix::preallocate_file` → `fcntl`
  (F_PREALLOCATE)** — the WAL's 8 MiB `reserve_space` chunks on APFS cost
  ~130 ms each (1.4 GiB WAL / 8 MiB ≈ 180 calls ≈ 22.5 s). This is the
  documented APFS extent-allocation pathology (findings/2026-08-22-rearm7),
  NOT a guest cost: the Linux container uses `fallocate(KEEP_SIZE)`,
  metadata-only. **Local WAL share does not transfer to the guest.**
- pwrite of WAL frames: ~10 % of samples. Real encode (fragment_from,
  crc32c hw) ≈ invisible. BTree memtable apply: 5 %.
- One sample window (84 %) sat in `Db::try_rotate_wal` → `Wal::create_on`
  → `open(2)` — rotation is per-flush (73 at 25M); its open+create is
  another APFS-heavy cost that hides inside wal_ms locally.

The guest's real hydrate attribution comes from run #23's existing diag:
**sum of FLUSHDUR = 63.35 s over 73 flushes (avg 867 ms ≈ 89 MiB/s) vs
hydrate wall 73.6 s.** Guest hydrate IS SST materialize (memtable walk →
block fill → lz4 → CRC → image write → install), serialized with ingestion
on the ~1-effective-core guest; commit-side CPU is small (prepare+mem+real
encode ≈ 2.9 s of 28.6 s locally, scaled ~12 s at 25M — and overlapped).

Consequences for RFC-0159:

1. **P1.1 as written ("bulk builder fills blocks straight from batch
   payload") targets the wrong path** — the bench materializes from parked
   memtables, not batch payloads. The real P1.1 = per-entry cuts in
   `write_sst_try_sorted_body` (double copy via `enc_scratch`, per-entry
   `InternalKey`/bloom clones, `Vec<Bytes>` bloom_keys rebuild).
2. lz4 runs on xorshift-random (incompressible) values — ~30 % of
   materialize CPU for ~10 % disk saving (5.75 GiB logical → 5.15 GiB on
   disk). Candidate: file-level compress-off when the first blocks probe
   incompressible (v3 uncompressed format already exists).
3. P1.2 (batch MANIFEST persists across chunk installs) attacks the
   per-chunk persist+fsync inside those 867 ms chunks.
4. P1.3 chunk size: 73 chunks × ~77 MiB (why not the 256 MiB buffer
   size?) — per-chunk fixed costs (open/fsync/manifest/bloom) × 3.3 more
   than a 256 MiB chunking would pay; run #19 showed bigger files also
   help read legs.

Next: env-gated FLUSHSTAGES breakdown (encode/lz4/bloom/write/fd) lands in
v24 so the guest split is measured, not extrapolated.
