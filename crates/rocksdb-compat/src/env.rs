//! rust-rocksdb `Env` stub. Pedra I/O is [`pedradb_core::Env`]; this type exists
//! so `BackupEngine::open(opts, &env)` compiles against the 0.22 names.

use super::Result;

/// rust-rocksdb `Env`. Construction succeeds; backup uses the DB's Pedra Env
/// for the checkpoint copy and `IoUringEnv` for the backup catalog.
#[derive(Debug, Clone, Default)]
pub struct Env {
    _priv: (),
}

impl Env {
    /// rust-rocksdb `Env::new` (default filesystem env).
    ///
    /// # Errors
    /// Never.
    pub fn new() -> Result<Self> {
        Ok(Self { _priv: () })
    }

    /// rust-rocksdb `Env::mem_env`. Pedra does not keep a separate mem env —
    /// same as [`Self::new`].
    ///
    /// # Errors
    /// Never.
    pub fn mem_env() -> Result<Self> {
        Self::new()
    }
}
