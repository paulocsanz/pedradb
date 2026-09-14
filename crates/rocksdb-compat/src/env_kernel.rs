//! rust-rocksdb 0.22 `Env` + `SstFileManager`.
//!
//! Pedra I/O is [`pedradb_core::Env`] (injected at open). This `Env` is the
//! rust-rocksdb **name**: `BackupEngine::open`, `Options::set_env`, thread-
//! pool knobs. Thread-pool setters are stored (compile + round-trip); Pedra
//! keeps its one host compact worker — extra Rocks pools do not spawn.
//! `mem_env` is the same default FS (no separate in-memory EnvFile).

use super::Result;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

/// rust-rocksdb `Env`.
#[derive(Debug, Clone, Default)]
pub struct Env {
    bg_threads: i32,
    high_threads: i32,
    low_threads: i32,
    bottom_threads: i32,
}

impl Env {
    /// rust-rocksdb `Env::new` (default filesystem env).
    ///
    /// # Errors
    /// Never.
    pub fn new() -> Result<Self> {
        Ok(Self::default())
    }

    /// rust-rocksdb `Env::mem_env`. Pedra does not keep a separate mem env —
    /// same as [`Self::new`].
    ///
    /// # Errors
    /// Never.
    pub fn mem_env() -> Result<Self> {
        Self::new()
    }

    /// rust-rocksdb `set_background_threads` (LOW pool). Stored; Pedra compact
    /// worker count is unchanged.
    pub fn set_background_threads(&mut self, num_threads: i32) {
        self.bg_threads = num_threads;
    }

    /// rust-rocksdb high-priority pool size. Stored; not spawned.
    pub fn set_high_priority_background_threads(&mut self, n: i32) {
        self.high_threads = n;
    }

    /// rust-rocksdb low-priority pool size. Stored; not spawned.
    pub fn set_low_priority_background_threads(&mut self, n: i32) {
        self.low_threads = n;
    }

    /// rust-rocksdb bottom-priority pool size. Stored; not spawned.
    pub fn set_bottom_priority_background_threads(&mut self, n: i32) {
        self.bottom_threads = n;
    }

    /// rust-rocksdb `join_all_threads`. No-op: we did not spawn Env pools.
    pub fn join_all_threads(&mut self) {}

    /// rust-rocksdb IO-priority lower. No-op (no Env pools).
    pub fn lower_thread_pool_io_priority(&mut self) {}

    /// rust-rocksdb high-pool IO-priority lower. No-op.
    pub fn lower_high_priority_thread_pool_io_priority(&mut self) {}

    /// rust-rocksdb CPU-priority lower. No-op.
    pub fn lower_thread_pool_cpu_priority(&mut self) {}

    /// rust-rocksdb high-pool CPU-priority lower. No-op.
    pub fn lower_high_priority_thread_pool_cpu_priority(&mut self) {}

    /// Last `set_background_threads` value (0 = unset).
    #[must_use]
    pub fn background_threads(&self) -> i32 {
        self.bg_threads
    }
}

/// rust-rocksdb / C++ `SstFileManager` — disk cap + delete rate for SST
/// files. rust-rocksdb 0.22 does **not** export this type; we do so a host
/// that names it (C++ docs, Surreal v2) compiles. Caps are stored; the
/// compact worker does not yet stall on them (rate = Inert until wired).
#[derive(Debug, Clone)]
pub struct SstFileManager {
    inner: Arc<SstInner>,
}

#[derive(Debug)]
struct SstInner {
    max_space: AtomicU64,
    compact_buf: AtomicU64,
    delete_rate: AtomicI64,
    total: AtomicU64,
}

impl SstFileManager {
    /// C++ `NewSstFileManager(env)`.
    ///
    /// # Errors
    /// Never.
    pub fn new(_env: &Env) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(SstInner {
                max_space: AtomicU64::new(0),
                compact_buf: AtomicU64::new(0),
                delete_rate: AtomicI64::new(0),
                total: AtomicU64::new(0),
            }),
        })
    }

    /// Max live SST bytes. `0` = unlimited.
    pub fn set_max_allowed_space_usage(&self, bytes: u64) {
        self.inner.max_space.store(bytes, Ordering::Relaxed);
    }

    /// Extra headroom while compaction is rewriting files.
    pub fn set_compaction_buffer_size(&self, bytes: u64) {
        self.inner.compact_buf.store(bytes, Ordering::Relaxed);
    }

    /// Trash-directory delete throttle (bytes/s). Stored; unlinks are not
    /// paced yet (Pedra tombstones, does not trash-delete SSTs).
    pub fn set_delete_rate_bytes_per_second(&self, n: i64) {
        self.inner.delete_rate.store(n, Ordering::Relaxed);
    }

    /// Tracked SST bytes (0 until a DB wires updates).
    #[must_use]
    pub fn get_total_size(&self) -> u64 {
        self.inner.total.load(Ordering::Relaxed)
    }

    /// True when [`Self::get_total_size`] ≥ max and max is not unlimited.
    #[must_use]
    pub fn is_max_allowed_space_reached(&self) -> bool {
        let max = self.inner.max_space.load(Ordering::Relaxed);
        max > 0 && self.inner.total.load(Ordering::Relaxed) >= max
    }
}
