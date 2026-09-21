//! rust-rocksdb `checkpoint::Checkpoint` on Pedra `ConcurrentDb::create_checkpoint`.

use std::marker::PhantomData;
use std::path::Path;

use pedradb_core::Env;
use pedradb_io_uring::IoUringEnv;

use super::{Error, Result, DB};

/// rust-rocksdb `Checkpoint`. Flush+file-set copy; dest is openable as a DB.
pub struct Checkpoint<'db, E: Env = IoUringEnv> {
    db: &'db DB<E>,
    _life: PhantomData<&'db ()>,
}

impl<'db, E: Env> Checkpoint<'db, E> {
    /// rust-rocksdb `Checkpoint::new`.
    ///
    /// # Errors
    /// Never — kept as `Result` for API shape.
    pub fn new(db: &'db DB<E>) -> Result<Self> {
        Ok(Self {
            db,
            _life: PhantomData,
        })
    }

    /// rust-rocksdb `create_checkpoint`. Dest must be empty or missing.
    ///
    /// # Errors
    /// Pedra checkpoint / I/O.
    pub fn create_checkpoint<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let dest = path.as_ref();
        self.db
            .inner
            .create_checkpoint(dest)
            .map(|_| ())
            .map_err(Error::from)?;
        crate::backup::copy_compat_sidecars(&self.db.inner.path(), dest)
    }
}
