// Rustc-linked Db open/recovery path (RFC-0157 stage 2).
// Included into `db_kernel.rs` (same module — private fields).
// Aeneas extracts the total plans this impl matches (`open_wal_head_plan`).

impl<E: Env> Db<E> {
    /// Open using a [`Host`]'s filesystem seam (`host.env()`).
    ///
    /// Clock/RNG on the host are unused by the kernel (leases / election live in
    /// layers); this is the one-call DST plug: `DetHost` + `FailingEnv`.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    pub fn open_with_host(
        path: impl AsRef<Path>,
        opts: OpenOptions,
        host: &impl Host<Env = E>,
    ) -> Result<Self> {
        Self::open_with_env(path, opts, host.env().clone())
    }

    /// Open with an explicit [`Env`] (fault injection, in-memory, …).
    ///
    /// Payloads of recovered SSTs stay fully resident (legacy behavior); use
    /// [`Self::open_with_env_bounded`] to bound residency.
    ///
    /// # Errors
    /// I/O failures, corrupt logical records, CRC errors, [`CoreError::AlreadyOpen`],
    /// or corrupt MANIFEST.
    pub fn open_with_env(path: impl AsRef<Path>, opts: OpenOptions, env: E) -> Result<Self> {
        Self::open_with_env_sourced(
            path,
            opts,
            env,
            None,
            Arc::new(crate::env::FileHandleCache::new(0)),
        )
    }

    /// Open with an explicit [`Env`] **and the SST payload pool armed**
    /// (RFC-0042 v18). Resident file bodies are held to
    /// `opts.sst_payload_budget_bytes` (default
    /// [`DEFAULT_SST_PAYLOAD_BUDGET_BYTES`]); eviction happens during
    /// recovery, so reopening a multi-GiB store does not transiently hold
    /// the whole dataset in RAM. Requires `E: Send + Sync + 'static` because
    /// evicted tables re-read their file through a shared
    /// [`SstFileSource`](crate::env::SstFileSource) built from the env.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    pub fn open_with_env_bounded(
        path: impl AsRef<Path>,
        mut opts: OpenOptions,
        env: E,
    ) -> Result<Self>
    where
        E: Env + Send + Sync + 'static,
        E::File: Send + 'static,
    {
        if opts.sst_payload_budget_bytes.is_none() {
            opts.sst_payload_budget_bytes = Some(sst_payload_budget_from_env());
        }
        if opts.large_value_threshold.is_none() {
            if let Ok(v) = std::env::var("PEDRA_MIN_BLOB_BYTES") {
                if let Ok(threshold) = v.trim().parse::<usize>() {
                    opts.large_value_threshold = Some(threshold);
                }
            }
        }
        let file_cache = Arc::new(crate::env::FileHandleCache::new(
            sst_file_cache_entries_from_env(),
        ));
        let source: Arc<dyn crate::env::SstFileSource> = Arc::new(
            crate::env::CachedEnvSource::new(<E as Clone>::clone(&env), Arc::clone(&file_cache)),
        );
        // Parallel merge executor for thread-shareable envs. Two opt-in
        // dimensions share the seam: within-job key-space spans
        // (`PEDRA_PARALLEL_MERGE=1` — guest run #17 neutral, local 6M A/B
        // net-negative: settle 7.9 vs 5.9 s, apply ~3× slower; stays off)
        // and across-job disjoint-batch compaction (`PEDRA_PARALLEL_JOBS`,
        // default 1 = off). Either being on needs the concrete executor
        // installed; the historical no-seam path (tests, generic envs)
        // merges sequentially either way.
        let jobs_k = parallel_jobs_from_env();
        let seam: Option<Arc<dyn ParallelMerge>> = if parallel_merge_enabled() || jobs_k > 1 {
            Some(Arc::new(ParallelMergeEnv::new(<E as Clone>::clone(&env))))
        } else {
            None
        };
        let mut db = Self::open_with_env_sourced(path, opts, env, Some(source), file_cache)?;
        if let Some(pm) = seam {
            db.set_parallel_merge(pm);
        }
        db.parallel_jobs = jobs_k;
        Ok(db)
    }

    /// Shared open path; `source = Some` arms the payload pool before
    /// recovery so reopen never materializes the whole dataset in RAM.
    ///
    /// # Errors
    /// Same as [`Self::open_with_env`].
    #[allow(clippy::too_many_lines)] // recover WAL + CHANGELOG + vlog in one open path
    fn open_with_env_sourced(
        path: impl AsRef<Path>,
        opts: OpenOptions,
        env: E,
        source: Option<Arc<dyn crate::env::SstFileSource>>,
        sst_file_cache: Arc<crate::env::FileHandleCache>,
    ) -> Result<Self> {
        crate::buggify_hooks::inject_checked(crate::buggify_hooks::sites::AFTER_OPEN_LOCK)?;
        let dir = path.as_ref().to_path_buf();
        env.create_dir_all(&dir)?;

        let lock = if opts.exclusive {
            Some(DirLock::acquire(&env, &dir)?)
        } else {
            None
        };

        manifest::cleanup_tmp_files(&env, &dir)?;

        let table_cache = TableCache::new(64);
        let block_cache = BlockCache::new(8192);
        let sst_payload_pool = Arc::new(crate::cache::SstPayloadPool::with_budget(
            opts.sst_payload_budget_bytes,
        ));
        if let Some(src) = &source {
            sst_payload_pool.arm();
            table_cache.set_payload_kit(crate::cache::PayloadKit {
                source: Arc::clone(src),
                pool: Arc::clone(&sst_payload_pool),
            });
        }
        // Official YCSB records=4096 (zipfian). 2048 FIFO + sequential load
        // evicted the hot low IDs; C then started cold (parkfold2 C 1.6×).
        let point_cache = Arc::new(PointCache::new(8192));
        let last_prefix_cache = AnswerCache::new(8192);
        let count_cache = Arc::new(crate::cache::CountCache::new(8192));
        let (
            ssts,
            sst_levels,
            next_file_num,
            manifest_file_num,
            vlog_use_new,
            mut max_seq,
            earliest_readable_seq,
        ) = recover_ssts(&env, &dir, opts.sync, &table_cache)?;

        let wal_path = dir.join(WAL_FILE_NAME);
        let mut mem = MemTable::new();
        let mut change_log = ChangeLog::load_on(&env, &dir)?;
        // RFC-0217 P1.1: archived WAL segments (rename-instead-of-truncate
        // while the lazy CHANGELOG cache lags). Their keys are durable in
        // SSTs listed by the MANIFEST **up to `manifest_floor`** — but the
        // deferred rotate publishes the MANIFEST only at the debounce
        // gate, so ops above the floor exist only in the archives
        // (unpublished SSTs are orphans, swept by `gc_orphan_ssts`).
        // Replay ops above the floor into the memtable as data; ops at or
        // below it are already in live SSTs and re-applying them would
        // resurrect superseded versions. The feed extension covers both.
        let manifest_floor = max_seq;
        let feed_max_loaded = change_log.max_sequence().unwrap_or(0);
        let mut wal_archives: Vec<u64> = env
            .read_dir_names(&dir)
            .unwrap_or_default()
            .iter()
            .filter_map(|n| wal_archive_slot_of(n))
            .collect();
        wal_archives.sort_unstable();
        let mut archive_max_seq_seen = 0u64;
        for slot in &wal_archives {
            let path = dir.join(wal_archive_slot_name(*slot));
            let (records, _last_good, _resync) = match Wal::recover_span_on(&env, &path) {
                Ok(r) => r,
                Err(e) => {
                    // A segment may be the only durable copy of the window
                    // above the manifest floor — torn frames are
                    // corruption, loud (never a silent skip).
                    return Err(crate::corrupt::escalate_or_fail(
                        &env,
                        &dir,
                        "arch",
                        0,
                        CoreError::Internal(format!(
                            "torn archived WAL segment {} ({e})",
                            path.display()
                        )),
                    ));
                }
            };
            for raw in records {
                let rec = WriteRecord::decode(&raw)?;
                if let Some(s) = rec.max_sequence() {
                    archive_max_seq_seen = archive_max_seq_seen.max(s);
                }
                // RFC-0217 P1.1: archived segments replay as data only
                // above the manifest floor — the rotate never archives a
                // below-floor window (see `unpublished_below_floor`), so
                // every dropped frame's entry is in a published SST, and a
                // replayed frame can never shadow newer published data
                // (`lookup` trusts mem-first).
                let data_ops: Vec<_> = rec
                    .ops
                    .iter()
                    .filter(|op| op.sequence > manifest_floor)
                    .cloned()
                    .collect();
                if !crate::write_admission_kernel::batch_is_empty(data_ops.len() as u64) {
                    apply_ops_owned(&mut mem, data_ops);
                    if let Some(s) = rec.max_sequence() {
                        max_seq = max_seq.max(s);
                    }
                }
                let mut missing = Vec::new();
                for op in &rec.ops {
                    if crate::write_admission_kernel::seq_after_feed(
                        op.sequence,
                        change_log.max_sequence().unwrap_or(0),
                    ) {
                        missing.push(ChangeEntry::from_write_op(op));
                    }
                }
                if !crate::write_admission_kernel::batch_is_empty(missing.len() as u64) {
                    change_log.extend(missing);
                }
            }
        }
        // RFC-0047 P0.2: set when a PointInTime open discards a WAL suffix.
        let mut point_in_time_report: Option<RecoveryReport> = None;

        let wal_exists = env.exists(&wal_path);
        match crate::write_admission_kernel::open_wal_head_plan(wal_exists, false, 0) {
            crate::write_admission_kernel::OpenWalHeadPlan::Skip => {}
            crate::write_admission_kernel::OpenWalHeadPlan::EmptyTiny
            | crate::write_admission_kernel::OpenWalHeadPlan::RecoverSpan => {
                let (records, last_good) = match Wal::recover_span_on(&env, &wal_path) {
                    Ok((records, last_good, resync_origin)) => {
                        if let Some(origin) = resync_origin {
                            // F171: a resync walk skipped damaged bytes and a
                            // later record re-anchored — the skipped region is
                            // lost from the recovered set. FailClosed refuses
                            // (never silent-wrong); PointInTime journals, reports
                            // and serves the recovered prefix.
                            let escalated = crate::corrupt::escalate_or_fail(
                                &env,
                                &dir,
                                "resync",
                                origin,
                                CoreError::Internal(
                                    "WAL resync skipped damaged region mid-log".into(),
                                ),
                            );
                            match crate::wal::reopen_kernel::reopen_outcome(
                                crate::wal::reopen_kernel::ReopenDamage::Resync,
                                opts.wal_recovery == WalRecovery::PointInTime,
                                matches!(escalated, CoreError::CorruptionEscalated { .. }),
                            ) {
                                crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                    let (records, last_good, _err, _resync) =
                                        Wal::recover_prefix_span_on(&env, &wal_path)?;
                                    point_in_time_report = Some(RecoveryReport {
                                        kind: "resync",
                                        corrupt_offset: origin,
                                        good_through_offset: last_good,
                                        discarded_bytes: env
                                            .metadata_len(&wal_path)
                                            .unwrap_or(0)
                                            .saturating_sub(last_good),
                                    });
                                    (records, last_good)
                                }
                                _ => return Err(escalated),
                            }
                        } else {
                            (records, last_good)
                        }
                    }
                    Err(CoreError::Truncated(0)) => {
                        let len = env.metadata_len(&wal_path).unwrap_or(0);
                        match crate::write_admission_kernel::open_wal_head_plan(true, true, len) {
                            crate::write_admission_kernel::OpenWalHeadPlan::EmptyTiny
                            | crate::write_admission_kernel::OpenWalHeadPlan::Skip => {
                                (Vec::new(), 0)
                            }
                            crate::write_admission_kernel::OpenWalHeadPlan::RecoverSpan => {
                                let escalated = crate::corrupt::escalate_or_fail(
                                    &env,
                                    &dir,
                                    "truncated_head",
                                    0,
                                    CoreError::Truncated(0),
                                );
                                // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                                match crate::wal::reopen_kernel::reopen_outcome(
                                    crate::wal::reopen_kernel::ReopenDamage::TruncatedHead,
                                    opts.wal_recovery == WalRecovery::PointInTime,
                                    matches!(escalated, CoreError::CorruptionEscalated { .. }),
                                ) {
                                    crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                        // Re-walk collecting the decoded prefix; the
                                        // stopping error is the same head error that
                                        // routed us here (first error is deterministic).
                                        let (records, last_good, _prefix_err, _resync) =
                                            Wal::recover_prefix_span_on(&env, &wal_path)?;
                                        point_in_time_report = Some(RecoveryReport {
                                            kind: "truncated_head",
                                            corrupt_offset: 0,
                                            good_through_offset: last_good,
                                            discarded_bytes: len.saturating_sub(last_good),
                                        });
                                        (records, last_good)
                                    }
                                    _ => return Err(escalated),
                                }
                            }
                        }
                    }
                    Err(e @ CoreError::Crc { offset, .. }) => {
                        // Mid-WAL bitflip: fail-stop (silent skip is G8-forbidden),
                        // journaled; the Nth event escalates (RFC-0038 D).
                        let escalated =
                            crate::corrupt::escalate_or_fail(&env, &dir, "crc", offset, e);
                        // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                        match crate::wal::reopen_kernel::reopen_outcome(
                            crate::wal::reopen_kernel::ReopenDamage::Crc,
                            opts.wal_recovery == WalRecovery::PointInTime,
                            matches!(escalated, CoreError::CorruptionEscalated { .. }),
                        ) {
                            crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                let (records, last_good, _prefix_err, _resync) =
                                    Wal::recover_prefix_span_on(&env, &wal_path)?;
                                point_in_time_report = Some(RecoveryReport {
                                    kind: "crc",
                                    corrupt_offset: offset,
                                    good_through_offset: last_good,
                                    discarded_bytes: env
                                        .metadata_len(&wal_path)
                                        .unwrap_or(0)
                                        .saturating_sub(last_good),
                                });
                                (records, last_good)
                            }
                            _ => return Err(escalated),
                        }
                    }
                    Err(e @ CoreError::WalZeroHeader { offset }) => {
                        // F170: zero header at a fresh alignment with a non-zero
                        // tail — corruption, never padding. Journaled (RFC-0038)
                        // so repeated events escalate; PointInTime serves the
                        // decoded prefix and reports the discard.
                        let escalated =
                            crate::corrupt::escalate_or_fail(&env, &dir, "zero_header", offset, e);
                        // RFC-0053 Y3.3: reopen outcome from the pure kernel.
                        match crate::wal::reopen_kernel::reopen_outcome(
                            crate::wal::reopen_kernel::ReopenDamage::ZeroHeader,
                            opts.wal_recovery == WalRecovery::PointInTime,
                            matches!(escalated, CoreError::CorruptionEscalated { .. }),
                        ) {
                            crate::wal::reopen_kernel::ReopenOutcome::ServePrefixReport => {
                                let (records, last_good, _prefix_err, _resync) =
                                    Wal::recover_prefix_span_on(&env, &wal_path)?;
                                point_in_time_report = Some(RecoveryReport {
                                    kind: "zero_header",
                                    corrupt_offset: offset,
                                    good_through_offset: last_good,
                                    discarded_bytes: env
                                        .metadata_len(&wal_path)
                                        .unwrap_or(0)
                                        .saturating_sub(last_good),
                                });
                                (records, last_good)
                            }
                            _ => return Err(escalated),
                        }
                    }
                    Err(e) => return Err(e),
                };
                match crate::write_admission_kernel::pit_resync_rewrite_plan(
                    point_in_time_report
                        .as_ref()
                        .is_some_and(|r| r.kind == "resync"),
                ) {
                    crate::write_admission_kernel::PitResyncRewritePlan::RewriteWalFromPrefix => {
                        assert!(
                            crate::write_admission_kernel::pit_resync_needs_rewrite(
                                point_in_time_report
                                    .as_ref()
                                    .is_some_and(|r| r.kind == "resync"),
                            ),
                            "resync report ⇒ rewrite WAL from recovered prefix"
                        );
                        let repair = dir.join(format!("{WAL_FILE_NAME}.repair"));
                        let mut w = Wal::create_on(&env, &repair)?;
                        w.set_full_fsync(opts.wal_full_fsync);
                        for raw in &records {
                            w.append_record(raw)?;
                        }
                        let sync_err = w.sync_data().err();
                        match crate::write_admission_kernel::wal_commit_plan(
                            true,
                            sync_err.is_some(),
                        ) {
                            crate::write_admission_kernel::WalCommitPlan::AppendSyncFence => {
                                assert!(
                                    crate::write_admission_kernel::fence_on_sync_fail(true, true),
                                    "required repair sync failed ⇒ not Ok"
                                );
                                return Err(sync_err.expect("AppendSyncFence ⇒ Some"));
                            }
                            crate::write_admission_kernel::WalCommitPlan::AppendSyncApplyOk
                            | crate::write_admission_kernel::WalCommitPlan::AppendApplyOk => {}
                        }
                        drop(w);
                        env.rename(&repair, &wal_path)?;
                        env.sync_dir(&dir)?;
                    }
                    crate::write_admission_kernel::PitResyncRewritePlan::KeepRecoveredPrefix => {}
                }
                let feed_max = change_log.max_sequence().unwrap_or(0);
                for raw in records {
                    let rec = WriteRecord::decode(&raw)?;
                    apply_record(&mut mem, &rec);
                    if let Some(s) = rec.max_sequence() {
                        max_seq = max_seq.max(s);
                    }
                    // Rebuild feed entries present in WAL but missing from CHANGELOG
                    // (crash between WAL sync and changelog persist).
                    let mut missing = Vec::new();
                    for op in &rec.ops {
                        if crate::write_admission_kernel::seq_after_feed(op.sequence, feed_max) {
                            missing.push(ChangeEntry::from_write_op(op));
                        }
                    }
                    if !crate::write_admission_kernel::batch_is_empty(missing.len() as u64) {
                        change_log.extend(missing);
                    }
                }
                let wal_len = env.metadata_len(&wal_path).unwrap_or(0);
                if crate::write_admission_kernel::torn_tail_needs_cut(wal_len, last_good) {
                    let mut wal_file = env.open_append(&wal_path)?;
                    wal_file.set_len(last_good)?;
                    let sync_err = wal_file.sync_data().err();
                    match crate::write_admission_kernel::wal_commit_plan(true, sync_err.is_some()) {
                        crate::write_admission_kernel::WalCommitPlan::AppendSyncFence => {
                            assert!(
                                crate::write_admission_kernel::fence_on_sync_fail(true, true),
                                "required torn-tail sync failed ⇒ not Ok"
                            );
                            return Err(sync_err.expect("AppendSyncFence ⇒ Some").into());
                        }
                        crate::write_admission_kernel::WalCommitPlan::AppendSyncApplyOk
                        | crate::write_admission_kernel::WalCommitPlan::AppendApplyOk => {}
                    }
                }
            }
        }

        // RFC-0217 P1.1: archives left behind by a debounced store are now
        // covered — the replay above folded every new archive op into the
        // feed (an unchanged feed means each archive op was already
        // last-per-key covered). Store once when anything extended.
        if crate::write_admission_kernel::seq_after_feed(
            change_log.max_sequence().unwrap_or(0),
            feed_max_loaded,
        ) {
            change_log.store_on(&env, &dir)?;
        }
        // RFC-0217 P1.1: the segments whose data the MANIFEST already
        // publishes (≤ manifest_floor) are redundant — delete now. Any
        // segment above the floor is the only durable copy of its window
        // (the memtable replay is volatile): keep the files; the running
        // db clears them at the next store point after the deferred
        // publish catches up.
        let keep_wal_archives = !wal_archives.is_empty() && manifest_floor < archive_max_seq_seen;
        if !keep_wal_archives {
            for slot in &wal_archives {
                let path = dir.join(wal_archive_slot_name(*slot));
                let _ = env.remove_file(&path);
            }
            if !wal_archives.is_empty() {
                let _ = env.sync_dir(&dir);
            }
        }

        let wal = Arc::new(Mutex::new({
            let mut w = if env.exists(&wal_path) {
                Wal::append_on(&env, &wal_path)?
            } else {
                Wal::create_on(&env, &wal_path)?
            };
            w.set_full_fsync(opts.wal_full_fsync);
            w
        }));

        // Watermark may exceed max sequence still present in SSTs (e.g. latest_only
        // dropped a high-seq tombstone). Keep last_sequence ≥ earliest so current
        // gets never look "too old" after reopen.
        let next_seq = max_seq.max(earliest_readable_seq).saturating_add(1).max(1);
        if crate::write_admission_kernel::seq_exhausted(next_seq, MAX_SEQUENCE_NUMBER) {
            return Err(CoreError::Internal(
                "sequence number space exhausted".into(),
            ));
        }

        let large_value_threshold = opts.large_value_threshold.filter(|n| *n > 0);
        let vlog_path = dir.join(VLOG_FILE_NAME);
        let vlog_new = dir.join(crate::vlog::VLOG_NEW_NAME);
        let blob_nums = vlog::list_blob_nums(&env, &dir);
        let blob_active = blob_nums.last().copied().unwrap_or(0);
        // RFC-0056 P1.4: vlog swing decision comes from the pure kernel; the
        // flag-resolved arms delegate to `open_with_flag`, which encodes the
        // same F51 refuse / empty-create rules as defense-in-depth.
        let vlog = match crate::vlog_gc_kernel::vlog_recover_action(
            blob_active > 0,
            large_value_threshold.is_some(),
            env.exists(&vlog_path),
            vlog_use_new,
            env.exists(&vlog_new),
        ) {
            crate::vlog_gc_kernel::VlogRecoverAction::NoVlog => None,
            crate::vlog_gc_kernel::VlogRecoverAction::OpenBlob => {
                Some(Arc::new(Mutex::new(ValueLog::open_blob(&env, &dir, blob_active)?)))
            }
            crate::vlog_gc_kernel::VlogRecoverAction::OpenNew
            | crate::vlog_gc_kernel::VlogRecoverAction::OpenPrimary
            | crate::vlog_gc_kernel::VlogRecoverAction::CreateEmptyPrimary
            | crate::vlog_gc_kernel::VlogRecoverAction::RefuseOpen => Some(Arc::new(Mutex::new(
                ValueLog::open_with_flag(&env, &dir, vlog_use_new)?,
            ))),
        };

        let mut db = Self {
            dir,
            env,
            wal,
            commit_inflight: Arc::new(AtomicUsize::new(0)),
            unapplied: Vec::new(),
            mem: LiveMem::from_table(mem),
            imm: None,
            imm_sv: None,
            sst_sv: std::sync::Arc::new(Vec::new()),
            flush_read_pin: None,
            parked_unflushed: Vec::new(),
            bulk_latch: crate::bulk_ingest::BulkLatch::new(),
            bulk_route_enabled: crate::bulk_ingest::bulk_enabled(),
            bulk_runs: HashMap::new(),
            parked_bulk: VecDeque::new(),
            bulk_encoding: None,
            bulk_manifest_debt: 0,
            fold_pair_expected: None,
            retired_pending: Vec::new(),
            retired_fold: MemTable::new(),
            retired_l0s: 0,
            sst_order_newest: Vec::new(),
            sst_runs: Vec::new(),
            ssts,
            sst_levels,
            physical_cfs: Vec::new(),
            next_file_num,
            manifest_file_num,
            manifest_epoch: 0,
            manifest_write_gate: Arc::new(Mutex::new(0)),
            vlog_use_new,
            next_seq: Arc::new(AtomicU64::new(next_seq)),
            published_seq: Arc::new(AtomicU64::new(next_seq.saturating_sub(1))),
            sync: opts.sync,
            auto_flush_bytes: opts.auto_flush_bytes.filter(|n| *n > 0),
            cf_write_buffer: std::collections::BTreeMap::new(),
            auto_compact_sst_count: opts.auto_compact_sst_count.filter(|n| *n > 0),
            auto_compact_sst_bytes: opts.auto_compact_sst_bytes.filter(|n| *n > 0),
            table_cache,
            block_cache,
            sst_payload_pool,
            sst_source: source,
            sst_file_cache,
            sst_page_keep_budget: 0,
            sst_warm_cap_bytes: std::env::var("PEDRA_SST_WARM_CAP_BYTES")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(crate::scale_kernel::WARM_FLOOR_BYTES),
            leftover_dontneed_issued: AtomicU64::new(0),
            scan_readahead_issued: AtomicU64::new(0),
            point_cache,
            last_prefix_cache,
            count_cache,
            read_cache_epoch: Arc::new(AtomicU64::new(1)),
            point_tls_epoch: Arc::new(AtomicU64::new(1)),
            key_gen: Arc::new(KeyGenMap::new()),
            phase_stats: std::env::var_os("PEDRA_WRITE_PHASE_STATS")
                .map(|_| Arc::new(WritePhaseStats::default())),
            last_recovery: point_in_time_report,
            dirty_points: Mutex::new(Vec::new()),
            point_cache_reset: AtomicBool::new(false),
            dir_lock: lock,
            durability_fenced: false,
            fence_report: None,
            occ_floor_registry: None,
            open_opts: opts,
            auto_compact_failures: 0,
            last_auto_compact_error: None,
            rewrite_chunk_target_bytes: REWRITE_CHUNK_TARGET_BYTES,
            large_value_threshold,
            vlog,
            vlog_rotate_bytes: None,
            blob_active,
            scan_prefetch: 4,
            prefetch_hits: AtomicU64::new(0),
            latest_ops: AtomicU64::new(0),
            latest_mem_hit: AtomicU64::new(0),
            latest_sst_fallback: AtomicU64::new(0),
            latest_sst_probed: AtomicU64::new(0),
            scan_ops: AtomicU64::new(0),
            scan_ns: AtomicU64::new(0),
            scan_sst_setup_ns: AtomicU64::new(0),
            scan_merge_ns: AtomicU64::new(0),
            scan_sst_probed: AtomicU64::new(0),
            get_mem_hit: AtomicU64::new(0),
            get_sst_fallback: AtomicU64::new(0),
            class_z0: Arc::new(AtomicU64::new(0)),
            class_z1: Arc::new(AtomicU64::new(0)),
            class_q: AtomicU64::new(0),
            class_w: Arc::new(AtomicU64::new(0)),
            hot_filter_pins: AtomicU64::new(0),
            get_inline: AtomicU64::new(0),
            get_vlog: AtomicU64::new(0),
            mvcc_split_ops: AtomicU64::new(0),
            mvcc_ns_encode: AtomicU64::new(0),
            mvcc_ns_last: AtomicU64::new(0),
            mvcc_ns_get: AtomicU64::new(0),
            mvcc_ns_copy: AtomicU64::new(0),
            auto_blob_gc_min_ratio: None,
            auto_reclaim: false,
            defer_auto_compact: false,
            write_stall_l0: None,
            write_stall_mem_bytes: None,
            write_pressure_l0: None,
            write_stall_drain: false,
            write_stall_count: 0,
            write_pressure_count: 0,
            max_ram_bytes: crate::ram_pressure_kernel::resolve_ram_budget(
                None,
                pedradb_posix::total_physical_memory_bytes(),
            ),
            ram_pressure_throttle_count: 0,
            snapshot_pins: std::collections::BTreeMap::new(),
            next_snapshot_pin_id: 1,
            earliest_readable_seq,
            history: opts.history,
            seq_times: Mutex::new(std::collections::VecDeque::new()),
            seq_time_counter: AtomicU64::new(0),
            history_tier: None,
            remote_history: None,
            uploaded_history_segs: std::collections::HashSet::new(),
            upload_bandwidth: None,
            remote_read_cache: Mutex::new(SegmentCache {
                budget: 64 * 1024 * 1024,
                ..Default::default()
            }),
            last_archive_millis: None,
            last_horizon_reclaim: None,
            wal_sync_count: AtomicU64::new(0),
            bytes_ingested: 0,
            bytes_written_wal: 0,
            bytes_written_sst: 0,
            compact_count: 0,
            vlog_gc_count: 0,
            changelog_disk_watermark: change_log.max_sequence().unwrap_or(0),
            manifest_published_seq: manifest_floor,
            // Recovery published (or adopted) exactly this inventory; the
            // WAL-less watermarks start clean (RFC-0217 P1.1).
            manifest_dirty: false,
            unpublished_below_floor: false,
            walless_seq_high: 0,
            wal_archive_max_seq: if keep_wal_archives {
                archive_max_seq_seen
            } else {
                0
            },
            wal_archive_next: if keep_wal_archives {
                wal_archives.last().map_or(0, |s| s.saturating_add(1))
            } else {
                0
            },
            wal_archive_unlinked: 0,
            change_log,
            changelog_interval: changelog_interval_from_env(),
            changelog_rebuild_budget_entries:
                crate::changelog_kernel::DEFAULT_CHANGELOG_REBUILD_BUDGET_ENTRIES,
            changelog_flushes_since_store: 0,
            wal_archive_cap: crate::changelog_kernel::DEFAULT_WAL_ARCHIVE_SEGMENT_CAP,
            changelog_flush_debounce:
                crate::changelog_kernel::DEFAULT_CHANGELOG_FLUSH_DEBOUNCE_FLUSHES,
            compact_target_file_bytes: crate::compact_kernel::COMPACT_TARGET_FILE_BYTES,
            l1_target_bytes: crate::compact_kernel::COMPACT_TARGET_FILE_BYTES,
            parallel_merge: None,
            parallel_jobs: 1,
            commits_since_changelog: 0,
            changelog_store_count: 0,
            disk_pressure_log: AtomicU8::new(0),
            disk_probe_cache_ms: std::env::var_os("PEDRA_DISK_PROBE_CACHE_MS")
                .and_then(|v| v.to_string_lossy().parse().ok())
                // RFC-0233 P1.3: default 0 was a `statvfs` per 1c put
                // (WRITEPHASE prepare 3 µs). Fjall does not probe the
                // filesystem on every insert. 1000 ms reuses an Ok verdict.
                .unwrap_or(1000),
            disk_ok_probe_at: None,
            disk_ladder_at: None,
            unsynced_ssts: Vec::new(),
        };
        // RFC-0042 v18: `ScanAndInstall` recovery (legacy dirs, no MANIFEST)
        // opens tables outside the table cache — attach + register them here
        // so the armed pool bounds them too.
        if let Some(src) = &db.sst_source {
            for t in &db.ssts {
                t.attach_payload_kit(src, &db.sst_payload_pool);
            }
        }
        db.rebuild_sst_order();
        db.maybe_rebuild_feed_from_live();
        // RFC-0046 P0.2: restore the archive floor across reopens (a cap
        // overflow in a previous life must keep failing old snaps closed).
        let tier = crate::history::HistoryTier::open(&db.env, &db.dir)?;
        db.raise_earliest_readable(tier.archive_floor());
        db.history_tier = Some(tier);
        Ok(db)
    }
}
