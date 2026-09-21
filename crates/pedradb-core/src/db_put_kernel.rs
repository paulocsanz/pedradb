// Rustc-linked Db put/apply/commit path (RFC-0157 stage 2).
// Included into `db_kernel.rs` (same module — private fields).
// Aeneas extracts the total plans this impl matches (`put_handler_plan`).

impl<E: Env> Db<E> {
    /// Put `key → value` (auto-commit, one sequence).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.put_with(key, value, WriteOptions::default())?;
        Ok(())
    }

    /// Put and return the assigned commit sequence (RFC-0019 P0.2 layer pin).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with_seq(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_with(key, value, WriteOptions::default())
    }

    /// Put with explicit [`WriteOptions`]; returns the commit sequence of the write.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn put_with(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_MEM_INSERT)?;
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_WAL_APPEND)?;
        self.apply_batch_with([BatchOp::put(key, value)], opts)
    }

    /// Put only if `key` has no live value (RFC-0019 CAS / LWT substitute).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] if the key already exists; WAL I/O otherwise.
    pub fn put_if_absent(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_absent_with(key, value, WriteOptions::default())
    }

    /// [`put_if_absent`](Self::put_if_absent) with [`WriteOptions`].
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_absent_with(
        &mut self,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.ensure_not_fenced()?;
        let k = key.as_ref();
        if !crate::write_admission_kernel::cas_absent_put(self.get(k).is_some()) {
            return Err(CoreError::CasMismatch);
        }
        self.put_with(k, value, opts)
    }

    /// Put `value` only if the live value equals `expected` (RFC-0019 CAS).
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] if missing or different; WAL I/O otherwise.
    pub fn put_if_eq(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_eq_with(key, expected, value, WriteOptions::default())
    }

    /// [`put_if_eq`](Self::put_if_eq) with [`WriteOptions`].
    ///
    /// # Errors
    /// [`CoreError::CasMismatch`] or WAL I/O.
    pub fn put_if_eq_with(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.ensure_not_fenced()?;
        let k = key.as_ref();
        let live_eq = match self.get(k) {
            Some(cur) => cur.as_ref() == expected.as_ref(),
            None => false,
        };
        if !crate::write_admission_kernel::cas_eq_put(live_eq) {
            return Err(CoreError::CasMismatch);
        }
        self.put_with(k, value, opts)
    }

    /// Alias for [`put_if_eq`](Self::put_if_eq) (compare-and-swap).
    ///
    /// # Errors
    /// Same as [`put_if_eq`](Self::put_if_eq).
    pub fn compare_and_swap(
        &mut self,
        key: impl AsRef<[u8]>,
        expected: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> Result<SequenceNumber> {
        self.put_if_eq(key, expected, value)
    }

    /// Delete `key` (auto-commit tombstone).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> Result<()> {
        self.delete_with(key, WriteOptions::default())?;
        Ok(())
    }

    /// Delete and return the tombstone sequence (RFC-0019 P0.2).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete_with_seq(&mut self, key: impl AsRef<[u8]>) -> Result<SequenceNumber> {
        self.delete_with(key, WriteOptions::default())
    }

    /// Delete with explicit [`WriteOptions`]; returns the commit sequence.
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn delete_with(
        &mut self,
        key: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<SequenceNumber> {
        self.apply_batch_with([BatchOp::delete(key)], opts)
    }

    /// Range-delete `[start, end)` (end exclusive). Keys outside remain.
    ///
    /// Implemented as a range tombstone in the WAL/MemTable; compaction with
    /// [`CompactOptions::latest_only`] drops covered keys.
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or `start >= end`.
    pub fn delete_range(&mut self, start: impl AsRef<[u8]>, end: impl AsRef<[u8]>) -> Result<()> {
        self.delete_range_with(start, end, WriteOptions::default())
    }

    /// [`delete_range`](Self::delete_range) with [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O, sequence exhaustion, or invalid bounds.
    pub fn delete_range_with(
        &mut self,
        start: impl AsRef<[u8]>,
        end: impl AsRef<[u8]>,
        opts: WriteOptions,
    ) -> Result<()> {
        let s = start.as_ref();
        let e = end.as_ref();
        if crate::write_admission_kernel::range_inverted(s >= e) {
            return Err(CoreError::Internal(
                "delete_range requires start < end".into(),
            ));
        }
        self.apply_batch_with([BatchOp::delete_range(s, e)], opts)?;
        Ok(())
    }

    /// Point lookups for many keys at the latest snapshot (RFC-0019 P1.1).
    ///
    /// Order matches `keys`; each entry is the same as [`Self::get`] for that
    /// key — including its fail-stop-on-corruption contract (F1).
    ///
    /// # Panics
    /// If any stored value is a vlog reference whose payload fails CRC/I-O.
    #[must_use]
    pub fn multi_get(&self, keys: &[impl AsRef<[u8]>]) -> Vec<Option<Bytes>> {
        keys.iter().map(|k| self.get(k.as_ref())).collect()
    }

    /// [`multi_get`](Self::multi_get) at an explicit [`Snapshot`].
    #[must_use]
    /// Multi-get at an explicit snapshot.
    ///
    /// Each key answers exactly as [`Self::get_at`] — including the
    /// below-watermark tier read and LSM fallback (RFC-0046 P2.1/P2.3):
    /// one per-key loop, one visibility contract.
    ///
    /// # Errors
    /// [`CoreError::SnapshotTooOld`] if `snap` is below the version-GC
    /// watermark and history for a requested key cannot be covered by the
    /// retained tier or a surviving LSM version.
    pub fn multi_get_at(
        &self,
        snap: Snapshot,
        keys: &[impl AsRef<[u8]>],
    ) -> Result<Vec<Option<Bytes>>> {
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            out.push(self.get_at(snap, k.as_ref())?);
        }
        Ok(out)
    }

    /// Changes with `from_seq < sequence <= to_seq` (RFC-0019 change feed).
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] when a live entry's vlog payload fails
    /// CRC/I-O during a lazy rebuild (F1: corruption is an error, never a
    /// raw pointer served as the user value).
    /// [`CoreError::SnapshotTooOld`] when the window starts below the
    /// retention watermark (RFC-0046): versions below it — including lone
    /// tombstones — may have been GC'd out of every source, so a partial
    /// `Ok` would silently drop events. Catch up from `earliest_readable`
    /// (or the last sequence a previous feed call returned) instead.
    pub fn changes(
        &self,
        from_seq: SequenceNumber,
        to_seq: SequenceNumber,
    ) -> Result<Vec<ChangeEntry>> {
        let first = from_seq.saturating_add(1);
        if crate::lookup_kernel::snap_below_watermark(first, self.earliest_readable_seq) {
            return Err(CoreError::SnapshotTooOld {
                requested: first,
                earliest: self.earliest_readable_seq,
            });
        }
        let to = to_seq.min(self.last_sequence());
        let entries = if self.feed_is_lazy() {
            self.lazy_feed_entries()?
                .into_iter()
                .filter(|e| e.sequence > from_seq && e.sequence <= to)
                .collect::<Vec<_>>()
        } else {
            self.change_log.changes_in(from_seq, to)
        };
        self.resolve_feed_entries(entries)
    }

    /// All durable changes with `sequence > from_seq` (tail / watch catch-up).
    ///
    /// Below the retention watermark (RFC-0046) this returns what survived
    /// GC — fine for the last-write-wins seeding callers use it for (the
    /// newest version of every key always survives GC, and a dropped lone
    /// tombstone leaves the key absent, which is the same final state),
    /// NOT exact event history. For an exact windowed feed use
    /// [`Self::changes`], which fails `SnapshotTooOld` below the watermark.
    ///
    /// Fail-stop on a corrupt value log (F1): the `Vec` shape cannot express
    /// the error and a swallowed resolve would serve the raw VLG pointer as
    /// the user value. Use [`Self::changes`] for an error-shaped read.
    ///
    /// # Panics
    /// If a live entry's vlog payload fails CRC/I/O.
    #[must_use]
    pub fn changes_after(&self, from_seq: SequenceNumber) -> Vec<ChangeEntry> {
        let from = from_seq.min(self.last_sequence());
        let entries = if self.feed_is_lazy() {
            match self.lazy_feed_entries() {
                Ok(e) => e,
                Err(e) => fail_stop_corrupt_value("changes_after feed rebuild", &e),
            }
            .into_iter()
            .filter(|e| e.sequence > from)
            .collect::<Vec<_>>()
        } else {
            self.change_log.changes_after(from)
        };
        match self.resolve_feed_entries(entries) {
            Ok(e) => e,
            // F1 contract (doc above): never serve the raw pointer; fail-stop.
            Err(e) => fail_stop_corrupt_value("changes_after feed resolve", &e),
        }
    }

    /// F190: cached feed entries are stored-form values (vlog pointer or the
    /// F188 inline escape) — resolve to user values at the read boundary so
    /// every feed path agrees with `collect_feed_from_live` (which resolves).
    /// Range-delete entries carry the exclusive end key, not a value: as-is.
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] when a spilled payload fails CRC/I/O.
    fn resolve_feed_entries(&self, entries: Vec<ChangeEntry>) -> Result<Vec<ChangeEntry>> {
        let mut out = Vec::with_capacity(entries.len());
        for mut e in entries {
            if e.kind == ChangeKind::Put {
                e.value = self.resolve_stored_value(e.value)?;
            }
            out.push(e);
        }
        Ok(out)
    }

    /// `changelog_interval == 0`: do not grow an in-memory ChangeEntry vec on
    /// every write (RFC-0039 P0.3 / RFC-0041 P1.1). Watchers rebuild last-per-key
    /// from mem+SST; flush/close still persist.
    fn feed_is_lazy(&self) -> bool {
        self.changelog_interval == 0
    }

    /// Live entry count the lazy CHANGELOG rebuild would materialize
    /// (mem + imm + every SST). Same view `collect_feed_from_live` walks.
    fn live_entry_estimate(&self) -> u64 {
        let mem = self.mem.len() as u64;
        let imm = self.imm.as_ref().map_or(0, |m| m.len() as u64);
        let ssts: u64 = self.ssts.iter().map(|t| t.len() as u64).sum();
        mem + imm + ssts
    }

    /// Full WAL history when the log is still live; last-per-key after rotate.
    ///
    /// # Errors
    /// [`CoreError::CorruptValue`] via [`Self::collect_feed_from_live`].
    fn lazy_feed_entries(&self) -> Result<Vec<ChangeEntry>> {
        let from_wal = self.collect_feed_from_wal();
        // F183: the WAL tail only covers writes since the last rotate. Keys
        // already flushed to SST + persisted CHANGELOG were silently dropped
        // by the WAL-first short-circuit (`put A; flush; put B` → feed `[B]`).
        // Union instead: flush-time last-per-key cache + newer WAL ops, in
        // sequence order.
        if !crate::write_admission_kernel::batch_is_empty(self.change_log.len() as u64) {
            let mut out = self.change_log.changes_after(0);
            if crate::write_admission_kernel::batch_is_empty(from_wal.len() as u64) {
                return Ok(out);
            }
            // Per-key cutoff, not `out.last().sequence`. The cache is
            // last-per-key sorted by seq, so the global max is some other
            // key's latest — WAL ops for a key whose cached latest is older
            // (snapshot wipe Delete, then export Put) were dropped, and
            // watchers/oracles saw a delete while `get` served the restore
            // (F-found World seed 502514).
            let mut cached_latest = std::collections::BTreeMap::<bytes::Bytes, u64>::new();
            for e in &out {
                cached_latest
                    .entry(e.key.clone())
                    .and_modify(|s| *s = (*s).max(e.sequence))
                    .or_insert(e.sequence);
            }
            out.extend(
                from_wal
                    .into_iter()
                    .filter(|e| e.sequence > cached_latest.get(&e.key).copied().unwrap_or(0)),
            );
            out.sort_by_key(|e| e.sequence);
            return Ok(out);
        }
        if !crate::write_admission_kernel::batch_is_empty(from_wal.len() as u64) {
            return Ok(from_wal);
        }
        self.collect_feed_from_live()
    }

    fn collect_feed_from_wal(&self) -> Vec<ChangeEntry> {
        let path = self.dir.join(WAL_FILE_NAME);
        if !self.env.exists(&path) {
            return Vec::new();
        }
        let Ok((records, _, _resync)) =
            crate::wal::Wal::<E::File>::recover_span_on(&self.env, &path)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for raw in records {
            let Ok(rec) = WriteRecord::decode(&raw) else {
                break;
            };
            for op in rec.ops {
                out.push(ChangeEntry::from_write_op(&op));
            }
        }
        out
    }

    /// # Errors
    /// [`CoreError::CorruptValue`] when a live entry's vlog payload fails
    /// CRC/I-O (F1: never serve the raw pointer as the user value).
    fn collect_feed_from_live(&self) -> Result<Vec<ChangeEntry>> {
        let mut latest: BTreeMap<Bytes, (InternalKey, Bytes)> = BTreeMap::new();
        let consider = |map: &mut BTreeMap<Bytes, (InternalKey, Bytes)>,
                        ik: InternalKey,
                        v: Bytes| match map.get(&ik.user_key) {
            Some((old, _)) if old.sequence >= ik.sequence => {}
            _ => {
                map.insert(ik.user_key.clone(), (ik, v));
            }
        };
        {
            let mem = self.mem.read();
            for (ik, v) in mem.iter_internal() {
                consider(&mut latest, ik.clone(), v.clone());
            }
        }
        if let Some(ref imm) = self.imm {
            for (ik, v) in imm.iter_internal() {
                consider(&mut latest, ik.clone(), v.clone());
            }
        }
        for sst in &self.ssts {
            // Streaming keeps L0 lazy (`entries_cloned` filled the
            // materialize cache on every explicit flush and broke the
            // lazy-input invariant of streaming L0 compact).
            let mut stream = sst.iter_internal_streaming();
            loop {
                match stream.next_entry() {
                    Ok(Some((ik, v))) => consider(&mut latest, ik, v),
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            table = %sst.path().display(),
                            "CHANGELOG feed rebuild: corrupt SST skipped (feed rebuilt on open)"
                        );
                        break;
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(latest.len());
        for (ik, v) in latest.into_values() {
            let value = self.resolve_stored_value(v.clone())?;
            out.push(ChangeEntry {
                sequence: ik.sequence,
                key: ik.user_key,
                kind: ChangeKind::from_value_type(ik.kind),
                value,
            });
        }
        // Hardening (B2, latent): last-per-key comes out in user-key order,
        // not sequence order. Every current caller re-sorts via
        // `replace_sorted`, but the feed contract is non-decreasing sequence
        // (watcher cursors) — enforce it at the source.
        out.sort_by_key(|e| e.sequence);
        Ok(out)
    }

    /// When CHANGELOG is missing after flush (WAL already truncated), rebuild a
    /// last-per-key feed from MemTable ∪ SSTs so fold/journal are not empty.
    fn maybe_rebuild_feed_from_live(&mut self) {
        let feed_empty = self.change_log.max_sequence().unwrap_or(0) == 0;
        if !changelog_needs_sst_rebuild(feed_empty, self.last_sequence()) {
            return;
        }
        // Same rebuild budget as the flush path: above it the cache stays
        // empty and `lazy_feed_entries` rebuilds on demand (WAL / live).
        if !crate::changelog_kernel::changelog_rebuild_within_budget(
            self.live_entry_estimate(),
            self.changelog_rebuild_budget_entries,
        ) {
            return;
        }
        // Best-effort cache rebuild: a corrupt payload keeps the (stale but
        // WAL-covered) feed as-is instead of failing the flush.
        let entries = match self.collect_feed_from_live() {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "CHANGELOG rebuild skipped: value-log read failed");
                return;
            }
        };
        if crate::write_admission_kernel::batch_is_empty(entries.len() as u64) {
            return;
        }
        self.change_log.replace_sorted(entries);
        self.persist_changelog_best_effort();
    }

    /// Apply an ordered multi-op batch atomically (one WAL record, no OCC).
    ///
    /// For Raft/log apply and bulk import: sequences are assigned in order;
    /// either the whole batch is durable on success or none of it is visible
    /// after recovery.
    ///
    /// Returns the sequence of the last op in the batch (or current
    /// `last_sequence` if the batch is empty).
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch(
        &mut self,
        ops: impl IntoIterator<Item = BatchOp>,
    ) -> Result<SequenceNumber> {
        self.apply_batch_with(ops, WriteOptions::default())
    }

    /// [`apply_batch`](Self::apply_batch) with [`WriteOptions`].
    ///
    /// # Errors
    /// WAL I/O or sequence exhaustion.
    pub fn apply_batch_with(
        &mut self,
        batch: impl IntoIterator<Item = BatchOp>,
        durability: WriteOptions,
    ) -> Result<SequenceNumber> {
        let do_sync = crate::write_admission_kernel::wal_sync_required(
            durability.sync.is_some(),
            durability.sync.unwrap_or(false),
            self.sync,
        );
        let mut iter = batch.into_iter();
        let first = match iter.next() {
            None => {
                let _ = crate::write_admission_kernel::put_handler_plan(0, false);
                return Ok(self.last_sequence());
            }
            Some(op) => op,
        };
        // RFC-0235 Endure W: a non-empty committed batch is a write.
        self.class_w.fetch_add(1, Ordering::Relaxed);
        // RFC-0233 P1.3: 1-op async — no Vec. `put_handler_plan` still gates.
        let second = iter.next();
        let n = 1u64.saturating_add(u64::from(second.is_some()));
        if crate::write_admission_kernel::one_op_commit(n) && !do_sync {
            match crate::write_admission_kernel::put_handler_plan(1, false) {
                crate::write_admission_kernel::PutHandlerPlan::EmptyOk => {
                    return Ok(self.last_sequence());
                }
                crate::write_admission_kernel::PutHandlerPlan::CommitThenFlush
                | crate::write_admission_kernel::PutHandlerPlan::RestoreSeqOnCommitErr => {
                    let seq_checkpoint = self.next_seq.load(Ordering::Relaxed);
                    return match self.commit_async_one(first) {
                        Ok(seq) => Ok(seq),
                        Err(e) => {
                            self.next_seq.store(seq_checkpoint, Ordering::Relaxed);
                            Err(e)
                        }
                    };
                }
            }
        }
        let mut batch = Vec::with_capacity(4);
        batch.push(first);
        if let Some(op) = second {
            batch.push(op);
        }
        batch.extend(iter);
        self.ensure_disk_pressure_admitted(false)?;
        if !self.write_admission_idle() {
            let families = self.batch_families(&batch);
            self.ensure_write_admitted_for(&families)?;
        }
        self.observe_bulk_batch(&batch);
        let seq_checkpoint = self.next_seq.load(Ordering::Relaxed);
        let mut records = Vec::new();
        for op in batch {
            let seq = match self.alloc_seq() {
                Ok(s) => s,
                Err(e) => {
                    self.next_seq.store(seq_checkpoint, Ordering::Relaxed);
                    return Err(e);
                }
            };
            match op {
                BatchOp::Put { key, value } => {
                    self.bytes_ingested = self.bytes_ingested.saturating_add(value.len() as u64);
                    let stored = match self.maybe_spill_large_value(value) {
                        Ok(v) => v,
                        Err(e) => {
                            self.next_seq.store(seq_checkpoint, Ordering::Relaxed);
                            return Err(e);
                        }
                    };
                    records.push(WriteOp::put(seq, key, stored));
                }
                BatchOp::Delete { key } => {
                    records.push(WriteOp::delete(seq, key));
                }
                BatchOp::DeleteRange { start, end } => {
                    records.push(WriteOp::delete_range(seq, start, end));
                }
            }
        }
        let n = records.len() as u64;
        match crate::write_admission_kernel::put_handler_plan(n, false) {
            crate::write_admission_kernel::PutHandlerPlan::EmptyOk => {
                return Ok(self.last_sequence());
            }
            crate::write_admission_kernel::PutHandlerPlan::CommitThenFlush
            | crate::write_admission_kernel::PutHandlerPlan::RestoreSeqOnCommitErr => {
                match self.commit_ops_with(records, durability) {
                    Ok(()) => {
                        assert!(
                            matches!(
                                crate::write_admission_kernel::put_handler_plan(n, false),
                                crate::write_admission_kernel::PutHandlerPlan::CommitThenFlush
                            ),
                            "commit Ok ⇒ CommitThenFlush"
                        );
                        // F18: the write is already durable (WAL fsync under sync=true). Auto-flush
                        // is a background space concern — failing it must not surface as "put/commit
                        // failed" or clients will retry and the operator loses the success signal.
                        self.maybe_auto_flush_best_effort();
                        Ok(self.last_sequence())
                    }
                    Err(e) => {
                        assert!(
                            matches!(
                                crate::write_admission_kernel::put_handler_plan(n, true),
                                crate::write_admission_kernel::PutHandlerPlan::RestoreSeqOnCommitErr
                            ),
                            "commit Err ⇒ RestoreSeqOnCommitErr"
                        );
                        self.next_seq.store(seq_checkpoint, Ordering::Relaxed);
                        Err(e)
                    }
                }
            }
        }
    }

    /// Flush WAL according to open options (for tests / graceful shutdown).
    ///
    /// After a series of `WriteOptions::no_sync()` writes, call this to make
    /// them durable (group fsync).
    ///
    /// # Errors
    /// I/O from fsync, or [`CoreError::DurabilityFenced`].
    pub fn sync(&mut self) -> Result<()> {
        self.ensure_not_fenced()?;
        let sync_err = self.wal.lock().sync_data().err();
        let failed = sync_err.is_some();
        match crate::write_admission_kernel::wal_commit_plan(true, failed) {
            crate::write_admission_kernel::WalCommitPlan::AppendSyncFence => {
                assert!(
                    crate::write_admission_kernel::fence_on_sync_fail(true, failed),
                    "required sync failed ⇒ fence, not Ok"
                );
                let e = sync_err.expect("AppendSyncFence ⇒ Some");
                self.fence_durability(&e, FenceClass::of_core(&e));
                Err(e)
            }
            crate::write_admission_kernel::WalCommitPlan::AppendSyncApplyOk
            | crate::write_admission_kernel::WalCommitPlan::AppendApplyOk => {
                self.note_wal_sync();
                Ok(())
            }
        }
    }

    pub(crate) fn commit_ops_with(
        &mut self,
        records: Vec<WriteOp>,
        durability: WriteOptions,
    ) -> Result<()> {
        self.ensure_not_fenced()?;
        if let Some(op) = records.first() {
            self.maybe_park_foreign_one_slash(op.key.as_ref());
        }
        let do_sync = crate::write_admission_kernel::wal_sync_required(
            durability.sync.is_some(),
            durability.sync.unwrap_or(false),
            self.sync,
        );
        self.vlog_prepare_wal(do_sync)?;
        let n = self.wal.lock().append_write_ops(&records)?;
        self.bytes_written_wal = self.bytes_written_wal.saturating_add(n);
        let planned = crate::write_admission_kernel::wal_commit_plan(do_sync, false);
        let sync_err = match planned {
            crate::write_admission_kernel::WalCommitPlan::AppendApplyOk => None,
            crate::write_admission_kernel::WalCommitPlan::AppendSyncApplyOk
            | crate::write_admission_kernel::WalCommitPlan::AppendSyncFence => {
                self.wal.lock().sync_data().err()
            }
        };
        match crate::write_admission_kernel::wal_commit_plan(do_sync, sync_err.is_some()) {
            crate::write_admission_kernel::WalCommitPlan::AppendSyncFence => {
                assert!(
                    crate::write_admission_kernel::fence_on_sync_fail(do_sync, true),
                    "required sync failed ⇒ fence, not Ok"
                );
                let e = sync_err.expect("AppendSyncFence ⇒ Some");
                self.durability_fenced = true;
                return Err(e);
            }
            crate::write_admission_kernel::WalCommitPlan::AppendSyncApplyOk => {
                self.note_wal_sync();
            }
            crate::write_admission_kernel::WalCommitPlan::AppendApplyOk => {}
        }
        // In-memory change feed after durable WAL. CHANGELOG on disk is a cache:
        // never gate commit success on a second fsync/rename (RFC-0019) — reopen
        // rebuilds missing entries from WAL. Always apply mem once WAL is durable
        // so get and feed stay aligned and sequences are not rolled back.
        // Bytes::clone is a refcount — payload is not memcpy'd again.
        // interval=0: do not grow a million-entry Vec on the apply path
        // (RFC-0041 P1.1); changes() rebuilds last-per-key from live tables.
        if !self.feed_is_lazy() {
            self.change_log
                .extend(records.iter().map(ChangeEntry::from_write_op));
        }
        // RFC-0219 P0.1: the durable-commit CHANGELOG fate (count toward
        // the debounce iff the commit was synced) is the kernel's decision
        // — the write-admission sync resolution the trampoline used to
        // make inline. The store itself stays here (I/O is trampoline).
        match crate::changelog_kernel::changelog_durable_commit_fate(
            durability.sync.is_some(),
            durability.sync.unwrap_or(false),
            self.sync,
        ) {
            crate::changelog_kernel::ChangelogCommitFate::Count => {
                self.maybe_persist_changelog_after_durable_commit();
            }
            crate::changelog_kernel::ChangelogCommitFate::Skip => {}
        }
        self.apply_ops_to_mem(records);
        Ok(())
    }
}
