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
use std::path::Path;

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
    #[cfg(all(target_os = "linux", target_env = "gnu", not(miri)))]
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

/// Unprivileged free bytes on the filesystem that holds `path`
/// (`statvfs` `f_bavail * f_frsize`).
///
/// Probe, not a durability barrier. Callers map `Err` to unknown and must
/// not treat a failed probe as disk-full (RFC-0179).
///
/// # Errors
/// `statvfs` failed, `path` contains an interior NUL, or the platform has
/// no `statvfs` (Windows / Miri).
pub fn filesystem_available_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(all(unix, not(miri)))]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "path contains interior NUL")
        })?;
        let mut buf = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `c_path` is a live CString; `buf` is written only on rc==0.
        // Signature is POSIX `int statvfs(const char *, struct statvfs *)`.
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), buf.as_mut_ptr()) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: rc==0 — the kernel initialized `buf`.
        let st = unsafe { buf.assume_init() };
        // `f_frsize` is `c_ulong`; `f_bavail` is `fsblkcnt_t`. Width is
        // platform-dependent — keep the `u64` cast even when it is a no-op.
        #[allow(clippy::unnecessary_cast)]
        {
            let frsize = st.f_frsize as u64;
            let bavail = st.f_bavail as u64;
            Ok(bavail.saturating_mul(frsize))
        }
    }
    #[cfg(not(all(unix, not(miri))))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "statvfs not available on this platform",
        ))
    }
}

/// WAL mmap grow quantum (RFC-0233 P1.3). Remap-per-append was 0.10× vs
/// Fjall; Fjall pre-sizes the journal to 64 MiB (`PRE_ALLOCATED_BYTES`)
/// once. One MiB keeps small tests off a 64 MiB `read` while still
/// covering a 1c YCSB-A cell in a single map.
pub const WAL_MAP_GROW: u64 = 1024 * 1024;

/// RFC-0233 P1.3: `MAP_SHARED` memcpy into the WAL file (page cache,
/// process-crash class = FlushWAL). Avoids a `write`/`pwrite` syscall per
/// frame. Mapping is cached per `(st_dev, st_ino)`; grow is
/// [`WAL_MAP_GROW`] aligned. No `msync`: the memcpy already dirties the
/// shared pages. Logical WAL size stays in `WalWriter::position`; trailing
/// zeros are `WalZeroHeader` (recover stop).
///
/// # Errors
/// `mmap` / `ftruncate` failure.
#[cfg(all(unix, not(miri)))]
#[derive(Clone, Copy)]
struct TlsMap {
    fd: i32,
    gen: u64,
    ptr: *mut u8,
    cap: u64,
}

#[cfg(all(unix, not(miri)))]
fn map_gen() -> &'static std::sync::atomic::AtomicU64 {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    &GEN
}

#[cfg(all(unix, not(miri)))]
thread_local! {
    static LAST_MAP: std::cell::Cell<TlsMap> = std::cell::Cell::new(TlsMap {
        fd: -1,
        gen: 0,
        ptr: std::ptr::null_mut(),
        cap: 0,
    });
}

#[cfg(all(unix, not(miri)))]
pub fn mapped_pwrite(file: &File, buf: &[u8], at: u64) -> io::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::Ordering;
    let fd = file.as_raw_fd();
    let need = at.saturating_add(buf.len() as u64);
    let lock = maps_rwlock();

    // Read domain: serve the memcpy from the TLS cache or an existing
    // slot. Grow / release take the write side, so the mapping cannot be
    // munmapped mid-copy (RFC-0237 P0.1: the off-lock PwriteJob made the
    // writer's segment grow concurrent with this call).
    {
        let r = lock.read().unwrap_or_else(|e| e.into_inner());
        let gen = map_gen().load(Ordering::Acquire);
        let hit = LAST_MAP.with(|c| {
            let l = c.get();
            if l.fd == fd && l.gen == gen && l.cap >= need && !l.ptr.is_null() {
                Some(l.ptr)
            } else {
                None
            }
        });
        if let Some(p) = hit {
            // SAFETY: TLS copy passed the gen check under this read guard;
            // `grow_map`/`mapped_release` cannot munmap it while held.
            unsafe {
                std::ptr::copy_nonoverlapping(buf.as_ptr(), p.add(at as usize), buf.len());
            }
            return Ok(());
        }
        let meta = file.metadata()?;
        let key = (meta.dev(), meta.ino());
        if let Some(slot) = r.get(&key) {
            if slot.fd == fd && slot.cap >= need && !slot.ptr.is_null() {
                let ptr = slot.ptr;
                let cap = slot.cap;
                LAST_MAP.with(|c| {
                    c.set(TlsMap { fd, gen, ptr, cap });
                });
                // SAFETY: slot mapping of at least `need` bytes; read guard
                // keeps grow / release out for the copy.
                unsafe {
                    std::ptr::copy_nonoverlapping(buf.as_ptr(), ptr.add(at as usize), buf.len());
                }
                return Ok(());
            }
        }
    }

    // Write domain: first mapping or growth. `grow_map` bumps `MAP_GEN`
    // before its munmap so other threads' TLS copies fail their gen check
    // instead of using a freed pointer.
    {
        let mut w = lock.write().unwrap_or_else(|e| e.into_inner());
        let meta = file.metadata()?;
        let key = (meta.dev(), meta.ino());
        let slot = w.entry(key).or_insert(MapSlot {
            ptr: std::ptr::null_mut(),
            cap: 0,
            fd,
        });
        slot.fd = fd;
        if slot.cap < need {
            grow_map(file, slot, align_map_cap(need))?;
        }
        if slot.ptr.is_null() || slot.cap < need {
            return Err(io::Error::other("mapped_pwrite grow left a short map"));
        }
        let ptr = slot.ptr;
        let cap = slot.cap;
        let gen = map_gen().load(Ordering::Acquire);
        LAST_MAP.with(|c| {
            c.set(TlsMap { fd, gen, ptr, cap });
        });
        // SAFETY: write guard held — no concurrent grow / release, and the
        // slot covers `need`.
        unsafe {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), ptr.add(at as usize), buf.len());
        }
    }
    Ok(())
}

/// Drop the cached mapping for `file` so a later `set_len` to the logical
/// frontier cannot SIGBUS a stale map (WAL close / Drop).
#[cfg(all(unix, not(miri)))]
pub fn mapped_release(file: &File) {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::Ordering;
    map_gen().fetch_add(1, Ordering::Release);
    let fd = file.as_raw_fd();
    LAST_MAP.with(|c| {
        let l = c.get();
        if l.fd == fd {
            c.set(TlsMap {
                fd: -1,
                gen: 0,
                ptr: std::ptr::null_mut(),
                cap: 0,
            });
        }
    });
    let Ok(meta) = file.metadata() else {
        return;
    };
    let key = (meta.dev(), meta.ino());
    let mut maps = maps_rwlock().write().unwrap_or_else(|e| e.into_inner());
    if let Some(slot) = maps.remove(&key) {
        unsafe {
            unmap_slot(&slot);
        }
    }
}

#[cfg(not(all(unix, not(miri))))]
pub fn mapped_release(_file: &File) {}

#[cfg(all(unix, not(miri)))]
fn align_map_cap(need: u64) -> u64 {
    if need == 0 {
        return WAL_MAP_GROW;
    }
    let q = need / WAL_MAP_GROW;
    let r = need % WAL_MAP_GROW;
    if r == 0 {
        need
    } else {
        q.saturating_add(1).saturating_mul(WAL_MAP_GROW)
    }
}

#[cfg(all(unix, not(miri)))]
struct MapSlot {
    ptr: *mut u8,
    cap: u64,
    fd: i32,
}

// Raw pointer cache: mmap regions are Send; we only copy bytes under the
// map lock (read guard for copies, write guard for grow/release), so the
// slot itself is never mutated or dereferenced outside a guard.
#[cfg(all(unix, not(miri)))]
unsafe impl Send for MapSlot {}
#[cfg(all(unix, not(miri)))]
unsafe impl Sync for MapSlot {}

#[cfg(all(unix, not(miri)))]
fn maps_rwlock(
) -> &'static std::sync::RwLock<std::collections::HashMap<(u64, u64), MapSlot>> {
    static MAPS: std::sync::OnceLock<
        std::sync::RwLock<std::collections::HashMap<(u64, u64), MapSlot>>,
    > = std::sync::OnceLock::new();
    MAPS.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

#[cfg(all(unix, not(miri)))]
unsafe fn unmap_slot(slot: &MapSlot) {
    if !slot.ptr.is_null() && slot.cap > 0 {
        libc::munmap(slot.ptr.cast(), slot.cap as usize);
    }
}

#[cfg(all(unix, not(miri)))]
fn grow_map(file: &File, slot: &mut MapSlot, new_cap: u64) -> io::Result<()> {
    // Pad i_size to the mapping (Fjall `set_len(PRE_ALLOCATED_BYTES)`).
    // Recover treats a zero header as end-of-log; Wal Drop truncates to
    // the logical frontier so closed files are not left padded.
    file.set_len(new_cap)?;
    // RFC-0237 P0.1: kill TLS copies *before* the munmap below — a stale
    // `ptr` would be a use-after-munmap for a concurrent off-lock writer.
    map_gen().fetch_add(1, std::sync::atomic::Ordering::Release);
    unsafe {
        unmap_slot(slot);
        slot.ptr = std::ptr::null_mut();
        slot.cap = 0;
        // SAFETY: POSIX mmap; `fd` is a live File; length is the ftruncated size.
        let p = libc::mmap(
            std::ptr::null_mut(),
            new_cap as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            slot.fd,
            0,
        );
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        slot.ptr = p.cast();
        slot.cap = new_cap;
    }
    Ok(())
}

#[cfg(not(all(unix, not(miri))))]
pub fn mapped_pwrite(file: &File, buf: &[u8], at: u64) -> io::Result<()> {
    let _ = (file, buf, at);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "mapped_pwrite requires unix",
    ))
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
    fn rfc0233_mapped_pwrite_honors_offset() {
        let dir = temp_dir();
        let path = dir.join("map.bin");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        mapped_pwrite(&f, b"CD", 2).unwrap();
        mapped_pwrite(&f, b"AB", 0).unwrap();
        drop(f);
        assert_eq!(std::fs::read(&path).unwrap()[..4], *b"ABCD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0237 P0.1 regression: the off-lock `PwriteJob` made
    /// `mapped_pwrite` concurrent with the writer's segment `grow_map`
    /// (munmap + remap per `WAL_MAP_GROW`). A thread whose TLS cache
    /// passed the fd+gen+cap check while another thread remapped used a
    /// freed pointer → SIGSEGV in the memcpy (25M deps_cache_overwrite_mc4
    /// repro, 2026-09-21). Copies now hold the read side of the map lock
    /// and `grow_map` bumps `MAP_GEN` before munmap. This test races four
    /// writers across quanta; on the pre-fix code it faults within
    /// seconds (the VM repro crashed <60 s).
    #[test]
    fn rfc0237_mapped_pwrite_survives_concurrent_grow() {
        let dir = temp_dir();
        let path = dir.join("race.bin");
        let f = std::sync::Arc::new(
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap(),
        );
        const THREADS: usize = 4;
        const ROUNDS: u64 = 8;
        const QUANTUM: u64 = WAL_MAP_GROW;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
        let mut joins = Vec::new();
        for t in 0..THREADS {
            let f = std::sync::Arc::clone(&f);
            let barrier = std::sync::Arc::clone(&barrier);
            joins.push(std::thread::spawn(move || {
                let pat = [0xA5u8 ^ t as u8; 64];
                barrier.wait();
                // Each thread writes its own stripes across several grow
                // quanta while the others do the same: forces interleaved
                // grow (munmap+remap) and stale-TLS reuse.
                for r in 0..ROUNDS {
                    for q in 0..QUANTUM / (pat.len() as u64) / 16 {
                        let at = q * (pat.len() as u64) * 16 * THREADS as u64
                            + r * QUANTUM
                            + (t as u64) * (pat.len() as u64);
                        mapped_pwrite(&f, &pat, at).unwrap();
                    }
                }
            }));
        }
        for j in joins {
            j.join().unwrap();
        }
        drop(f);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc0233_mapped_pwrite_grows_in_chunks_not_per_append() {
        let dir = temp_dir();
        let path = dir.join("chunk.bin");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        mapped_pwrite(&f, b"hello", 0).unwrap();
        mapped_pwrite(&f, b"world", 5).unwrap();
        let meta_len = f.metadata().unwrap().len();
        assert_eq!(
            meta_len, WAL_MAP_GROW,
            "two small appends must share one {WAL_MAP_GROW} map, not remap to exact need"
        );
        mapped_release(&f);
        drop(f);
        let got = std::fs::read(&path).unwrap();
        assert_eq!(&got[..10], b"helloworld");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS tooth: ignore rc");
        assert!(
            !fdatasync_eintr_retry_admitted(),
            "EINTR must not retry as Ok (RFC-0015 H1)"
        );
        assert!(
            fdatasync_eintr_retry_admitted_as_is(),
            "AS-IS tooth: swallow EINTR"
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

    #[test]
    fn filesystem_available_bytes_temp_dir_nonzero() {
        let dir = temp_dir();
        match filesystem_available_bytes(&dir) {
            Ok(n) => assert!(n > 0, "temp fs reported 0 free bytes"),
            Err(e) => {
                #[cfg(all(unix, not(miri)))]
                panic!("statvfs on temp dir failed: {e}");
                #[cfg(not(all(unix, not(miri))))]
                let _ = e;
            }
        }
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
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS tooth: ignore rc");
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
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS tooth: ignore rc");
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
        assert!(fdatasync_rc_ok_as_is(-1), "AS-IS tooth: ignore rc");
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
        let src = include_str!("lib_kernel.rs");
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
            "statvfs(",
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
            sites >= 6,
            "expected the 6 known production FFI rc sites, found {sites}"
        );
    }
}
