//! POSIX `fdatasync(2)` for PedraDB.
//!
//! `pedradb-core` is `#![forbid(unsafe_code)]`. On Apple, Rust's
//! [`std::fs::File::sync_data`] is `fcntl(F_FULLFSYNC)` (~5 ms here), while
//! RocksDB / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs).
//! This crate issues the real syscall so WAL commit can match that class
//! (RFC-0001 / RFC-0036) without skipping the barrier.

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
