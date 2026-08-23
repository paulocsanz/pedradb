//! POSIX durability / hint syscalls for PedraDB.
//!
//! `pedradb-core` is `#![forbid(unsafe_code)]`. On Apple, Rust's
//! [`std::fs::File::sync_data`] is `fcntl(F_FULLFSYNC)` (~5 ms here), while
//! RocksDB / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs).
//! This crate issues the real syscall so WAL commit can match that class
//! (RFC-0001 / RFC-0036) without skipping the barrier. macOS WAL
//! preallocation (`F_PREALLOCATE`) also lives here (`preallocate_file`).
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
    /// Linux `POSIX_FADV_WILLNEED`.
    WillNeed,
    /// Linux `POSIX_FADV_DONTNEED`.
    DontNeed,
}

/// `fdatasync(2)` on `file`'s data (not Apple `F_FULLFSYNC`).
///
/// # Errors
/// Underlying I/O.
pub fn fdatasync_file(file: &File) -> io::Result<()> {
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
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(unix))]
    {
        file.sync_data()
    }
}

/// Reserve `len` bytes of storage past the file's physical end (macOS
/// `fcntl(F_PREALLOCATE)`).
///
/// APFS assigns a new extent when a plain append crosses an ~8 MiB boundary;
/// that `write(2)` blocks 10–50 ms inside the commit path while the writer
/// waits on the metadata journal (measured: `findings/2026-08-22-rearm7/`).
/// Reserving space up front keeps appends µs-scale. RocksDB preallocates its
/// WAL the same way (`PosixWritableFile`). Blocks are allocated without
/// changing the logical size — readers never observe the reserved region
/// (WAL zero padding beyond EOF is unreachable through `len`).
///
/// Best-effort on other platforms (no-op `Ok(())`): Linux ext4/xfs delayed
/// allocation does not block appends this way; revisit if a Linux box
/// measures a comparable tail.
///
/// # Errors
/// Underlying I/O when the platform implements the reservation.
pub fn preallocate_file(file: &File, len: u64) -> io::Result<()> {
    #[cfg(target_os = "macos")]
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
    #[cfg(not(target_os = "macos"))]
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
    file.sync_all()
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
        // Linux `linux/fadvise.h`: WILLNEED=3, DONTNEED=4. Not Darwin
        // (no posix_fadvise). Avoid the `libc` crate so this island has
        // zero dependencies.
        const POSIX_FADV_WILLNEED: i32 = 3;
        const POSIX_FADV_DONTNEED: i32 = 4;
        let advice = match kind {
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
    fn sync_dir_fd_ok() {
        let dir = temp_dir();
        let d = File::open(&dir).unwrap();
        sync_dir_fd(&d).unwrap();
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
    /// Process crash after Ok is covered by WAL recover tests; drive-cache
    /// power-loss is the weaker class and cannot be simulated in-process.
    /// This test proves the *class*: file + dirfd barriers stay on the fast
    /// `fdatasync` side of `File::sync_all` (`F_FULLFSYNC`, ~100× here).
    #[cfg(target_os = "macos")]
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
}
