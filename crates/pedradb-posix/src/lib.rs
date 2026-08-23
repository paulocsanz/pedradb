//! POSIX `fdatasync(2)` for PedraDB.
//!
//! `pedradb-core` is `#![forbid(unsafe_code)]`. On Apple, Rust's
//! [`std::fs::File::sync_data`] is `fcntl(F_FULLFSYNC)` (~5 ms here), while
//! RocksDB / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs).
//! This crate issues the real syscall so WAL commit can match that class
//! (RFC-0001 / RFC-0036) without skipping the barrier. macOS WAL
//! preallocation (`F_PREALLOCATE`) also lives here (`preallocate_file`).

use std::fs::File;
use std::io;

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
        unsafe extern "C" {
            fn fdatasync(fd: i32) -> i32;
        }
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
        unsafe extern "C" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn fdatasync_file_ok() {
        let dir = std::env::temp_dir().join(format!(
            "pedra-posix-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"x").unwrap();
        fdatasync_file(&f).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
