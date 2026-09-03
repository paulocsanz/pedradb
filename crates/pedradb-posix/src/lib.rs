//! POSIX durability / hint syscalls for PedraDB.
//!
//! `pedradb-core` is `#![forbid(unsafe_code)]`. On Apple, Rust's
//! [`std::fs::File::sync_data`] is `fcntl(F_FULLFSYNC)` (~5 ms here), while
//! RocksDB / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs).
//! This crate issues the real syscall so WAL commit can match that class
//! (RFC-0001 / RFC-0036) without skipping the barrier. WAL space reservation
//! also lives here (`preallocate_file`): Darwin `F_PREALLOCATE`, Linux
//! `fallocate(FALLOC_FL_KEEP_SIZE)` — same class as Rocks
//! `PosixWritableFile::Allocate`.
//!
//! All `unsafe` in the workspace's default I/O path lives here. Callers see
//! only safe functions. Invariants: crate `SAFETY.md`.
//!
//! `unsafe extern` (Rust 1.82) is **not** used so workspace `rust-version`
//! 1.75 still builds (edition 2021). The `extern "C"` block's SAFETY comment
//! is the signature assertion.

use std::fs::File;
use std::io;

/// Kernel readahead / cache-drop hint ([`advise_file`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAdvise {
    /// Linux `POSIX_FADV_RANDOM` — disable readahead (Rocks
    /// `set_advise_random_on_open`, default true for SST).
    Random,
    /// Linux `POSIX_FADV_WILLNEED`.
    WillNeed,
    /// Linux `POSIX_FADV_DONTNEED`.
    DontNeed,
}

/// Admit a libc `fdatasync` return (RFC-0073). Nonzero is not Ok.
#[must_use]
pub fn fdatasync_rc_ok(rc: i32) -> bool {
    rc == 0
}

/// AS-IS: ignore rc (the 0073 hole — skip the barrier on EIO/EINTR).
#[must_use]
pub fn fdatasync_rc_ok_as_is(_rc: i32) -> bool {
    true
}

/// RFC-0073 P2.2 / RFC-0015 H1: retry `fdatasync` on EINTR until rc==0
/// and return Ok. Always false. One syscall; EINTR/`rc != 0` is Err
/// (uncertain: the record may already be on disk).
#[must_use]
pub fn fdatasync_eintr_retry_admitted() -> bool {
    false
}

/// AS-IS: swallow EINTR / loop until Ok (the H1 hole).
#[must_use]
pub fn fdatasync_eintr_retry_admitted_as_is() -> bool {
    true
}

fn posix_rc_to_io(rc: i32) -> io::Result<()> {
    if fdatasync_rc_ok(rc) {
        Ok(())
    } else if fdatasync_eintr_retry_admitted() {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `fdatasync(2)` on `file`'s data (not Apple `F_FULLFSYNC`).
///
/// `PEDRA_FDSYNC_DIAG=1` prints an aggregate line every 2048 barriers —
/// the **in-load** fd cost (idle probes understate it), the number that
/// bounds any write leg whose batches ack behind one barrier each.
///
/// # Errors
/// Underlying I/O.
pub fn fdatasync_file(file: &File) -> io::Result<()> {
    if !fdsync_diag_enabled() {
        return fdatasync_file_inner(file);
    }
    let t0 = std::time::Instant::now();
    let out = fdatasync_file_inner(file);
    let us = t0.elapsed().as_micros() as u64;
    static NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static MAX_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use std::sync::atomic::Ordering::Relaxed;
    NS.fetch_add(us * 1000, Relaxed);
    MAX_US.fetch_max(us, Relaxed);
    let n = N.fetch_add(1, Relaxed) + 1;
    // PEDRA_FDSYNC_CALLERS: print the call stack of every Nth barrier so a
    // sync storm can be attributed to its emitter (aggregate lines cannot).
    if let Ok(step) = std::env::var("PEDRA_FDSYNC_CALLERS") {
        if let Ok(step) = step.parse::<u64>() {
            if step > 0 && n % step == 0 {
                println!(
                    "FDSYNCCALLER n={n}\n{}",
                    std::backtrace::Backtrace::force_capture()
                );
            }
        }
    }
    if n % 2048 == 0 {
        println!(
            "FDSYNCDIAG n={n} cum_ms={} avg_us={:.0} max_ms={:.1}",
            NS.load(Relaxed) / 1_000_000,
            (NS.load(Relaxed) / 1000) / n,
            MAX_US.load(Relaxed) as f64 / 1000.0,
        );
    }
    out
}

fn fdsync_diag_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PEDRA_FDSYNC_DIAG").is_some())
}

fn fdatasync_file_inner(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // Apple: `libc` / rustix omit `fdatasync` (they want F_FULLFSYNC).
        // libSystem and Linux both export the POSIX symbol.
        // SAFETY: signature is POSIX `int fdatasync(int fd)` / libSystem.
        // Edition 2021: plain `extern "C"` (MSRV 1.75). The call is `unsafe`.
        extern "C" {
            fn fdatasync(fd: i32) -> i32;
        }
        // SAFETY:
        // - `file` is an open `std::fs::File`; `as_raw_fd()` is not stored.
        // - The linked symbol matches the extern signature above.
        // - Non-zero `rc` leaves errno on this thread for `last_os_error`.
        let rc = unsafe { fdatasync(file.as_raw_fd()) };
        posix_rc_to_io(rc)
    }
    #[cfg(not(unix))]
    {
        file.sync_data()
    }
}

/// Reserve `len` bytes of storage past the file's logical end without
/// changing `i_size`.
///
/// - **Darwin:** `fcntl(F_PREALLOCATE)` / `F_PEOFPOSMODE`. APFS assigns a
///   fresh extent when a plain append crosses an ~8 MiB boundary; that
///   `write(2)` blocks 10–50 ms inside the commit path
///   (`findings/2026-08-22-rearm7/`).
/// - **Linux:** `fallocate(FALLOC_FL_KEEP_SIZE)` from current `i_size`.
///   Delayed allocation is cheap on async `write`; G1 `fdatasync` of a
///   growing WAL still has to allocate extents on the Ok path. Rocks
///   `PosixWritableFile::Allocate` pays this up front — Pedra must too
///   (RFC-0062 P1.1 p11b: coluna B min 0.15 vs Rocks `sync=true`).
///
/// Recovery never observes the reserved region (reads stop at logical
/// `len`). Best-effort: unsupported FS (`EOPNOTSUPP`) returns `Ok`. Miri
/// no-ops (no `F_PREALLOCATE` / `fallocate`).
///
/// # Errors
/// Underlying I/O when the platform implements the reservation.
pub fn preallocate_file(file: &File, len: u64) -> io::Result<()> {
    if len == 0 {
        return Ok(());
    }
    #[cfg(all(target_os = "macos", not(miri)))]
    {
        use std::os::fd::AsRawFd;

        // sys/fcntl.h
        const F_PREALLOCATE: i32 = 42;
        const F_ALLOCATEALL: u32 = 0x0000_0004;
        // Allocate from physical EOF (past the logical size).
        const F_PEOFPOSMODE: i32 = 3;

        // struct fstore (sys/fcntl.h): flags, posmode, offset, length.
        #[repr(C)]
        struct Fstore {
            fst_flags: u32,
            fst_posmode: i32,
            fst_offset: i64,
            fst_length: i64,
        }

        // SAFETY: signature is POSIX `int fcntl(int, int, ...)`. The struct
        // layout mirrors `fstore_t`; non-zero `rc` sets errno for
        // `last_os_error`.
        extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        let st = Fstore {
            fst_flags: F_ALLOCATEALL,
            fst_posmode: F_PEOFPOSMODE,
            fst_offset: 0,
            fst_length: len as i64,
        };
        // SAFETY: `file` is an open `std::fs::File`; `as_raw_fd()` is not
        // stored; the pointer is valid for the duration of the call.
        let rc = unsafe { fcntl(file.as_raw_fd(), F_PREALLOCATE, &st as *const Fstore) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(all(target_os = "linux", not(miri)))]
    {
        use std::os::fd::AsRawFd;
        // linux/falloc.h — allocate past EOF without growing i_size.
        const FALLOC_FL_KEEP_SIZE: i32 = 0x01;
        // SAFETY: signature is Linux `int fallocate(int, int, off_t, off_t)`
        // with `off_t` = i64 on LP64 (the only targets we ship).
        extern "C" {
            fn fallocate(fd: i32, mode: i32, offset: i64, len: i64) -> i32;
        }
        let offset = i64::try_from(file.metadata()?.len()).unwrap_or(i64::MAX);
        let n = i64::try_from(len).unwrap_or(0);
        // SAFETY: `file` is an open `std::fs::File`; `as_raw_fd()` is not
        // stored; `mode` is the documented KEEP_SIZE flag.
        let rc = unsafe { fallocate(file.as_raw_fd(), FALLOC_FL_KEEP_SIZE, offset, n) };
        if rc == 0 {
            Ok(())
        } else {
            let err = io::Error::last_os_error();
            // NFS / some FUSE: reservation is an optimization, not a barrier.
            // 95 = EOPNOTSUPP, 38 = ENOSYS (linux/asm-generic/errno*.h).
            match err.raw_os_error() {
                Some(95 | 38) => Ok(()),
                _ => Err(err),
            }
        }
    }
    #[cfg(any(miri, not(any(target_os = "macos", target_os = "linux"))))]
    {
        let _ = (file, len);
        Ok(())
    }
}

/// Full metadata barrier (`fsync` / `FlushFileBuffers` / Apple `F_FULLFSYNC`
/// via std). Used for published SST / MANIFEST, not WAL G1.
///
/// # Errors
/// Underlying I/O.
pub fn fsync_file(file: &File) -> io::Result<()> {
    // RFC-0073 P1.1: Linux/other unix FFI `fsync` shares `fdatasync_rc_ok`.
    // Darwin stays `File::sync_all` (`F_FULLFSYNC`) — not this G1 class.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: signature is POSIX `int fsync(int fd)`.
        extern "C" {
            fn fsync(fd: i32) -> i32;
        }
        // SAFETY: `file` is an open `std::fs::File`; `as_raw_fd()` is not stored.
        let rc = unsafe { fsync(file.as_raw_fd()) };
        posix_rc_to_io(rc)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        file.sync_all()
    }
}

/// Directory-entry barrier at the **same class as WAL G1** (`fdatasync`, not
/// Apple `F_FULLFSYNC`). POSIX specifies `fdatasync` for regular-file data;
/// Linux treats `fdatasync(dirfd)` as a metadata sync in practice. Darwin
/// directory-fd semantics are weaker than `F_FULLFSYNC` by product choice
/// (RFC-0036).
///
/// # Errors
/// Underlying I/O.
pub fn sync_dir_fd(dir: &File) -> io::Result<()> {
    fdatasync_file(dir)
}

/// Linux `posix_fadvise(2)` on `file`. No-op elsewhere (hint, not a barrier).
///
/// `offset`/`len` that do not fit in `off_t` clamp; the kernel then sees a
/// best-effort range. Callers must not fail a user request solely because
/// this returns `Err`.
///
/// # Errors
/// Underlying I/O on Linux when the hint is rejected.
pub fn advise_file(file: &File, offset: u64, len: u64, kind: FileAdvise) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // Linux `linux/fadvise.h`: RANDOM=1, WILLNEED=3, DONTNEED=4.
        // Not Darwin (no posix_fadvise). Avoid the `libc` crate so this
        // island has zero dependencies.
        const POSIX_FADV_RANDOM: i32 = 1;
        const POSIX_FADV_WILLNEED: i32 = 3;
        const POSIX_FADV_DONTNEED: i32 = 4;
        let advice = match kind {
            FileAdvise::Random => POSIX_FADV_RANDOM,
            FileAdvise::WillNeed => POSIX_FADV_WILLNEED,
            FileAdvise::DontNeed => POSIX_FADV_DONTNEED,
        };
        // SAFETY: signature is Linux `int posix_fadvise(int, off_t, off_t, int)`
        // with `off_t` = i64 on LP64 (the only targets we ship).
        extern "C" {
            fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
        }
        let off = i64::try_from(offset).unwrap_or(i64::MAX);
        let n = i64::try_from(len).unwrap_or(0);
        // SAFETY: `file` is open for the call; `advice` is a `POSIX_FADV_*`
        // constant. Return value is an errno-style code (0 = success), not
        // `-1` + `errno`.
        let rc = unsafe { posix_fadvise(file.as_raw_fd(), off, n, advice) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(rc))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (file, offset, len, kind);
        Ok(())
    }
}

/// glibc `malloc_trim(0)`: release free arena pages back to the OS.
/// Used after whole-levels rewrite chunks — glibc pins freed small
/// chunks next to retained ones (per-block index keys), so RSS creeps
/// even though nothing is retained (the 6M macOS repro is flat; the
/// 25M glibc guest climb was monotonic). Advisory only: the rc (1 =
/// released something, 0 = nothing to release) is deliberately not a
/// barrier-style gate. No-op off Linux glibc.
pub fn trim_process_heap() {
    #[cfg(all(target_os = "linux", not(miri)))]
    {
        // SAFETY: signature is glibc `int malloc_trim(size_t pad)`. `0`
        // means release as much as possible; there is no errno contract.
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: no pointers, no stored state; rc is advisory.
        let _rc = unsafe { malloc_trim(0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("pedra-posix-{n}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fdatasync_file_ok() {
        let dir = temp_dir();
        let path = dir.join("w.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"x").unwrap();
        fdatasync_file(&f).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0073 P1.2: the rc gate is a safe predicate (Miri, no syscall).
    #[test]
    fn fdatasync_rc_ok_is_safe_predicate() {
        assert!(fdatasync_rc_ok(0));
        assert!(!fdatasync_rc_ok(-1));
        assert!(!fdatasync_rc_ok(1));
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS dente: ignore rc");
        assert!(
            !fdatasync_eintr_retry_admitted(),
            "EINTR must not retry as Ok (RFC-0015 H1)"
        );
        assert!(
            fdatasync_eintr_retry_admitted_as_is(),
            "AS-IS dente: swallow EINTR"
        );
    }

    /// RFC-0073 P2.2 / RFC-0015 H1: EINTR is not retried as Ok.
    #[test]
    fn fdatasync_eintr_is_not_retried_as_ok() {
        assert!(!fdatasync_eintr_retry_admitted());
        assert!(fdatasync_eintr_retry_admitted_as_is());
        assert!(!fdatasync_rc_ok(-1), "EINTR is typically rc=-1");
        let dir = temp_dir();
        let path = dir.join("h1.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"wal").unwrap();
        fdatasync_file(&f).expect("production path is one syscall, then rc gate");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0152 P2.2.36: production `fdatasync_file` gates rc through
    /// `fdatasync_rc_ok`. Live WAL file is Ok (rc==0); live pipe fd is
    /// Err (nonzero rc). AS-IS would skip the barrier. Direct
    /// `fdatasync_nonzero_rc_is_not_ok` / `fdatasync_rc_ok_is_safe_predicate`
    /// are not this tooth.
    #[test]
    fn fdatasync_rc_ok_on_live_posix_is_not_ok() {
        assert!(!fdatasync_rc_ok(-1));
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS dente: ignore rc");
        let dir = temp_dir();
        let path = dir.join("wal.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"wal").unwrap();
        fdatasync_file(&f).expect("live fdatasync_file rc==0 is Ok");
        // Miri: `fdatasync` is only supported on file-backed fds (RFC-0073
        // island script). Pipe ENOTSUP is a host-syscall tooth.
        #[cfg(all(unix, not(miri)))]
        {
            use std::os::fd::FromRawFd;
            extern "C" {
                fn pipe(fds: *mut i32) -> i32;
                fn close(fd: i32) -> i32;
            }
            let mut fds = [0i32; 2];
            // SAFETY: POSIX `pipe(2)`; both fds are open on rc==0.
            assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0);
            // SAFETY: `fds[1]` is the write end we own. `fdatasync` on a
            // pipe is ENOTSUP/EINVAL (nonzero rc). Drop closes the write end.
            let w = unsafe { File::from_raw_fd(fds[1]) };
            assert!(
                fdatasync_file(&w).is_err(),
                "live fdatasync_file nonzero rc is Err"
            );
            drop(w);
            unsafe {
                close(fds[0]);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0073 P0: production `fdatasync_file` on a real file; nonzero rc
    /// is not Ok. AS-IS would ignore the barrier error.
    #[test]
    fn fdatasync_nonzero_rc_is_not_ok() {
        assert!(fdatasync_rc_ok(0));
        assert!(!fdatasync_rc_ok(-1));
        assert!(!fdatasync_rc_ok(1));
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS dente: ignore rc");
        let dir = temp_dir();
        let path = dir.join("g1.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"wal").unwrap();
        fdatasync_file(&f).expect("production fdatasync_file must succeed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fsync_file_ok() {
        let dir = temp_dir();
        let path = dir.join("w.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"y").unwrap();
        fsync_file(&f).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preallocate_does_not_grow_logical_size() {
        let dir = temp_dir();
        let path = dir.join("wal.bin");
        let f = File::create(&path).unwrap();
        preallocate_file(&f, 0).unwrap();
        preallocate_file(&f, 1024 * 1024).unwrap();
        assert_eq!(
            f.metadata().unwrap().len(),
            0,
            "KEEP_SIZE / F_PREALLOCATE must not become visible WAL bytes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_dir_fd_ok() {
        let dir = temp_dir();
        let d = File::open(&dir).unwrap();
        sync_dir_fd(&d).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0073 P1.1: `fsync_file` / `sync_dir_fd` share `fdatasync_rc_ok`
    /// where they FFI. Live files succeed; nonzero rc is not Ok.
    #[test]
    fn fsync_and_dirfd_share_rc_gate() {
        assert!(!fdatasync_rc_ok(-1));
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS dente: ignore rc");
        let dir = temp_dir();
        let path = dir.join("g1.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"sst").unwrap();
        fsync_file(&f).expect("production fsync_file must succeed");
        let d = File::open(&dir).unwrap();
        sync_dir_fd(&d).expect("production sync_dir_fd must succeed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn advise_file_is_best_effort() {
        let dir = temp_dir();
        let path = dir.join("blob.bin");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&[0u8; 4096]).unwrap();
            f.sync_all().unwrap();
        }
        let f = File::open(&path).unwrap();
        advise_file(&f, 0, 0, FileAdvise::Random).unwrap();
        advise_file(&f, 0, 4096, FileAdvise::WillNeed).unwrap();
        advise_file(&f, 0, 4096, FileAdvise::DontNeed).unwrap();
        // Overflow into `off_t` clamps (hint, not a barrier). Must not panic
        // or hold a dangling fd: `as_raw_fd` is not stored.
        let _ = advise_file(&f, u64::MAX, u64::MAX, FileAdvise::WillNeed);
        let _ = advise_file(&f, u64::MAX, 0, FileAdvise::DontNeed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fdatasync_empty_and_after_write() {
        let dir = temp_dir();
        let path = dir.join("empty.bin");
        let mut f = File::create(&path).unwrap();
        fdatasync_file(&f).unwrap();
        f.write_all(b"abc").unwrap();
        fdatasync_file(&f).unwrap();
        fsync_file(&f).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Darwin G1 is libSystem `fdatasync`, **not** `F_FULLFSYNC` (RFC-0036).
    /// Host wall-clock class; Miri time is not the disk.
    /// Process crash after Ok is covered by WAL recover tests; drive-cache
    /// power-loss is the weaker class and cannot be simulated in-process.
    /// This test proves the *class*: file + dirfd barriers stay on the fast
    /// `fdatasync` side of `File::sync_all` (`F_FULLFSYNC`, ~100× here).
    #[cfg(all(target_os = "macos", not(miri)))]
    #[test]
    fn darwin_fdatasync_and_dirfd_are_not_fullfsync_class() {
        use std::time::{Duration, Instant};

        fn p50_ns(mut samples: Vec<Duration>) -> u128 {
            samples.sort();
            samples[samples.len() / 2].as_nanos()
        }

        let dir = temp_dir();
        let path = dir.join("wal.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(&[0u8; 4096]).unwrap();

        let mut fd = Vec::with_capacity(80);
        let mut ff = Vec::with_capacity(80);
        for i in 0..80 {
            f.write_all(&[i as u8; 64]).unwrap();
            let t = Instant::now();
            fdatasync_file(&f).unwrap();
            fd.push(t.elapsed());
            let t = Instant::now();
            f.sync_all().unwrap();
            ff.push(t.elapsed());
        }
        let fd_p50 = p50_ns(fd);
        let ff_p50 = p50_ns(ff);
        eprintln!(
            "darwin class: file fdatasync p50={fd_p50}ns  File::sync_all(F_FULLFSYNC) p50={ff_p50}ns"
        );
        assert!(
            fd_p50.saturating_mul(8) < ff_p50,
            "G1 must be fdatasync-class, not F_FULLFSYNC: fdatasync p50={fd_p50}ns sync_all p50={ff_p50}ns"
        );

        let d = File::open(&dir).unwrap();
        let mut dir_fd = Vec::with_capacity(40);
        let mut dir_ff = Vec::with_capacity(40);
        for _ in 0..40 {
            let t = Instant::now();
            sync_dir_fd(&d).unwrap();
            dir_fd.push(t.elapsed());
            let t = Instant::now();
            d.sync_all().unwrap();
            dir_ff.push(t.elapsed());
        }
        let dir_fd_p50 = p50_ns(dir_fd);
        let dir_ff_p50 = p50_ns(dir_ff);
        eprintln!(
            "darwin class: dirfd fdatasync p50={dir_fd_p50}ns  dir File::sync_all p50={dir_ff_p50}ns (dir sync_all class is noisy; not G1)"
        );
        // `sync_dir_fd` is stably the fast class. `File::sync_all` on a
        // Darwin dirfd is **not** a reliable FULLFSYNC (this host: ~300 ns
        // or ~5 ms depending on the run) — we do not use it for publish.
        assert!(
            dir_fd_p50.saturating_mul(8) < ff_p50,
            "dirfd fdatasync must not be file F_FULLFSYNC class: dir={dir_fd_p50}ns file FULLFSYNC={ff_p50}ns"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0156 P0.1 (R-unsafe-posix): every **production** `unsafe` FFI
    /// site that returns an rc must be gated in the same expression
    /// window (`posix_rc_to_io(rc)` / `rc == 0` / errno match). A new
    /// ungated site fails this test with its line number — the class
    /// "unchecked FFI rc" cannot re-enter silently. Test-only sites
    /// (this module) are out of the scanned region.
    #[test]
    fn posix_unsafe_rc_sites_all_gated() {
        let src = include_str!("lib.rs");
        let lines: Vec<&str> = src.lines().collect();
        let cut = lines
            .iter()
            .position(|l| l.contains("mod tests {"))
            .expect("tests module marker");
        let ffns = [
            "fdatasync(",
            "fcntl(",
            "fallocate(",
            "fsync(",
            "posix_fadvise(",
        ];
        let mut sites = 0usize;
        for (i, line) in lines[..cut].iter().enumerate() {
            if !line.contains("unsafe {") {
                continue;
            }
            if !ffns.iter().any(|f| line.contains(f)) {
                continue;
            }
            sites += 1;
            let window: Vec<&str> = lines[i..(i + 8).min(lines.len())].to_vec();
            let gated = window.iter().any(|w| {
                w.contains("posix_rc_to_io(rc)")
                    || w.contains("rc == 0")
                    || w.contains("rc != 0")
                    || w.contains("raw_os_error")
            });
            assert!(gated, "ungated unsafe FFI rc at line {}: {line}", i + 1);
        }
        assert!(
            sites >= 5,
            "expected the 5 known production FFI rc sites, found {sites}"
        );
    }
}
