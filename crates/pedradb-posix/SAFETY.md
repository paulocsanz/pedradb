# SAFETY — `pedradb-posix`

This crate is the **only** `unsafe` on Pedra's default I/O path
(`StdEnv` / WAL G1). Public functions are safe. Core stays
`#![forbid(unsafe_code)]`.

## Blocks

### `fdatasync` (`fdatasync_file`, Unix)

- `extern "C" { fn fdatasync(fd: i32) -> i32; }` — POSIX / libSystem
  `int fdatasync(int)`. Edition 2021 + workspace MSRV 1.75: the block is
  **not** `unsafe extern` (stabilized 1.82). The SAFETY comment on the
  block is the signature assertion.
- Call: `file` is a live `std::fs::File`; `as_raw_fd()` is not stored.
- `rc == 0` success via `fdatasync_rc_ok` (RFC-0073); else `Error::last_os_error()` (errno on this thread).
- Non-Unix: `File::sync_data()` (Windows `FlushFileBuffers`); no FFI.

### `fsync` (`fsync_file`, Unix except Darwin)

- `extern "C" { fn fsync(fd: i32) -> i32; }` — POSIX `int fsync(int)`.
- Same `fdatasync_rc_ok` gate as `fdatasync_file` (RFC-0073 P1.1).
- Darwin: `File::sync_all()` (`F_FULLFSYNC`); no FFI here.

### `sync_dir_fd`

- Calls `fdatasync_file` (same FFI + `fdatasync_rc_ok`). Not Darwin `F_FULLFSYNC`.

**Not `F_FULLFSYNC`.** On Darwin this is weaker than Rust std
`File::sync_data`. Same barrier class as the rust-rocksdb peer on this
host (RFC-0036). Power-loss can lose a “synced” WAL if the drive cache
holds it. Proof: test `darwin_fdatasync_and_dirfd_are_not_fullfsync_class`
(file `fdatasync` p50 ~25 µs vs `F_FULLFSYNC` ~4 ms; dirfd `fdatasync`
stays fast). `File::sync_all` on a Darwin **dirfd** is noisy — not used.

`EINTR` / failed `fdatasync` after the kernel may have completed the
barrier is the RFC-0015 H1 uncertain outcome — not unique to unsafe.
RFC-0073 P2.2: `fdatasync_eintr_retry_admitted` is always false (no
retry-as-Ok loop). `rc != 0` (including EINTR / `-1`) is `Err`.

### `posix_fadvise` (`advise_file`, Linux only)

- `extern "C" { fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32; }`
  — Linux LP64 `off_t` = `i64`. Advice constants: `WILLNEED=3`, `DONTNEED=4`
  (`linux/fadvise.h`). No `libc` crate dep.
- `file` open for the call. Overflow of `offset`/`len` into `off_t` clamps
  (hint, not a durability barrier).
- Return is an errno-style code (0 = success), **not** `-1` + errno.
- Non-Linux: no-op `Ok(())`. Darwin has no `posix_fadvise`.

### `preallocate_file` (Darwin `F_PREALLOCATE` / Linux `fallocate`)

- Live `File`; `as_raw_fd()` is not stored. Darwin `fstore_t` layout is local.
- **Linux:** `extern "C" { fn fallocate(int, int, off_t, off_t) -> i32; }`
  with `FALLOC_FL_KEEP_SIZE = 0x01`. Offset is current `i_size`; `len` is
  the reservation. `i_size` does not grow — WAL recovery never observes
  the reserved region. `EOPNOTSUPP` (95) / `ENOSYS` (38) map to `Ok`
  (reservation is an optimization).
- **Miri:** Darwin `F_PREALLOCATE` is unsupported (`fcntl` cmd 0x2a) and
  Linux `fallocate` is not interpreted — the function no-ops under
  `cfg(miri)`. Production still reserves extents. This is not a durability
  barrier.

### `fsync_file` / `sync_dir_fd`

No `unsafe`. `fsync_file` is `File::sync_all`. `sync_dir_fd` is
`fdatasync_file` on a directory fd (G1 class, RFC-0036) — not Darwin
`F_FULLFSYNC`. Directory-fd `fdatasync` is Linux-practical; Darwin is the
same product tradeoff as WAL.

## What this crate must not grow

- `mmap`, io_uring, C ABI handles.
- Linking `fdatasync` through rustix/libc on Apple (they omit the symbol).
