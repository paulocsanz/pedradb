// Rustc-linked ConcurrentDb put path (RFC-0157 stage 2).
// Included into concurrent_kernel.rs (same module).

impl<E: Env> ConcurrentDb<E> {
    /// Put via write group (may share fsync with concurrent writers).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put(&self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_with(key, value, WriteOptions::default())
    }

    /// Put with options via write group.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        self.put_with_seq(key, value, opts).map(|_| ())
    }

    /// Put via write group and return the commit sequence (RFC-0019 P0.2).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with_seq(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        // RFC-0051 P0: PCT preemption point at client op entry (lock-free).
        #[cfg(feature = "pct")]
        crate::pct_hooks::maybe_yield("op_entry");
        match crate::group_commit_kernel::lock_alphabet_linearizes_n2(
            crate::group_commit_kernel::LOCK_ACT_ACQUIRE_WRITE,
            crate::group_commit_kernel::LOCK_ACT_SUBMIT,
        ) {
            false => {
                return Err(CoreError::Internal(
                    "lock alphabet refused acquire-write then submit".into(),
                ));
            }
            true => {}
        }
        match crate::group_commit_kernel::lock_alphabet_linearizes_n3(
            crate::group_commit_kernel::LOCK_ACT_ACQUIRE_WRITE,
            crate::group_commit_kernel::LOCK_ACT_SUBMIT,
            crate::group_commit_kernel::LOCK_ACT_PUBLISH,
        ) {
            false => {
                return Err(CoreError::Internal(
                    "lock alphabet refused acquire-write, submit, publish".into(),
                ));
            }
            true => {}
        }
        let do_sync = self.resolve_sync(opts);
        self.assist_flush_debt();
        let seq = self
            .writes
            .submit_one(&self.inner, BatchOp::put(key, value), do_sync)?;
        // RFC-0235 WriteBurst: do not let L0 grow past the trigger.
        if self.workload_class_now() == crate::WorkloadClass::WriteBurst {
            let l0 = self.inner.read().level_file_count(0) as u64;
            if crate::flush_kernel::l0_compact_due(l0, crate::db::L0_COMPACTION_TRIGGER as u64) {
                let _ = self.compact_l0_off_lock();
            }
        }
        Ok(seq)
    }

    /// Put only if key is absent (atomic under write lock; RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_absent(
        &self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        // Hold write lock for get+put so concurrent CAS cannot race.
        self.inner.write().put_if_absent(key, value)
    }

    /// Put only if live value equals `expected` (RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_eq(
        &self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.inner.write().put_if_eq(key, expected, value)
    }
}
