// Rustc-linked ConcurrentDb open path (RFC-0157 stage 2).
// Included into concurrent_kernel.rs (same module).

impl<E: Env> ConcurrentDb<E> {
    /// Open with an explicit [`Env`].
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Ok(Self::from_db(Db::open_with_env(path, opts, env)?))
    }

    /// Open with an explicit [`Env`] and the SST payload pool armed
    /// (RFC-0042 v18) — see [`Db::open_with_env_bounded`].
    ///
    /// # Errors
    /// Same as [`Db::open_with_env`].
    pub fn open_with_env_bounded(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self>
    where
        E: Env + Send + Sync + 'static,
        E::File: Send + 'static,
    {
        let mut db = Db::open_with_env_bounded(path, opts, env.clone())?;
        db.set_parallel_merge(Arc::new(ParallelMergeEnv::new(env)));
        Ok(Self::from_db(db))
    }
}
