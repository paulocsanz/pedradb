//! History-tier / below-watermark get trampoline (RFC-0174 P2.2).
//! Submodule of `db` so `Db` private fields stay visible.
//! Not a decision kernel — Env/history I/O only.

use super::*;

impl<E: Env> Db<E> {
    /// RFC-0046 P2.1: point read below the version-GC watermark, served
    /// lazy from the history tier (local segments first, the remote mirror
    /// second — it retains what the local cap already dropped).
    ///
    /// A decisive record (newest put/delete/range-delete at `seq ≤ snap`)
    /// answers even when coverage has gaps. A no-match answers `None` only
    /// when the retained segments provably cover `[1, snap]` with nothing
    /// dropped — otherwise fail-closed [`CoreError::SnapshotTooOld`]
    /// (never-written and dropped are indistinguishable). Read cost: the
    /// P2.5 key-coverage bound and the P2.6/P2.7 bloom sidecar prune
    /// segments (local and remote alike) before their bytes are read;
    /// only may-affect segments are fetched and CRC-walked.
    pub(super) fn get_at_from_archive(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        let too_old = || CoreError::snapshot_too_old(snap.seq, self.earliest_readable_seq);
        let Some(tier) = self.history_tier.as_ref() else {
            return Err(too_old());
        };
        // Candidate segments: the local manifest plus (content-addressed
        // dedup by name) everything the remote manifest still lists.
        // Entries carry the P2.5 key-coverage bound (range-delete aware)
        // — segments whose bound excludes the key are skipped without a
        // walk. Remote listings carry the bound of their manifest
        // generation (v3+); a pre-P2.5 remote manifest decodes with
        // `None` (walk).
        #[allow(clippy::type_complexity)]
        let mut cands: Vec<(u64, u64, String, Option<u64>, Option<(Vec<u8>, Vec<u8>)>)> = tier
            .segment_metas()
            .into_iter()
            .map(|m| {
                (
                    m.from_seq,
                    m.through_seq,
                    m.name,
                    Some(m.id),
                    m.key_lo.zip(m.key_hi),
                )
            })
            .collect();
        if let Some(remote) = self.remote_history.as_ref() {
            for seg in remote.tier.latest_segments(&remote.env)? {
                if !cands.iter().any(|(_, _, name, _, _)| *name == seg.name) {
                    cands.push((
                        seg.from_seq,
                        seg.through_seq,
                        seg.name,
                        None,
                        seg.key_lo.zip(seg.key_hi),
                    ));
                }
            }
        }
        let mut best: Option<crate::history::HistoryRecord> = None;
        let mut missing_below_snap = false;
        for (from, _, name, local_id, coverage) in &cands {
            if *from > snap.seq {
                continue; // cannot hold a record this snapshot can see
            }
            if let Some((lo, hi)) = coverage {
                if key < lo.as_slice() || key > hi.as_slice() {
                    continue; // outside the segment's key coverage — sound skip
                }
            }
            // P2.6 bloom sidecar: skip the record walk when the segment
            // provably cannot decide this key (helps the overlapping-key
            // case the manifest bound can't prune). Coverage spans below
            // still use `cands`, so the None-proof is unaffected by skips.
            if let Some(id) = local_id {
                if !tier.segment_may_affect(&self.env, *id, key) {
                    continue;
                }
            }
            let records = match local_id.map(|id| tier.read_local_segment(&self.env, id)) {
                Some(Ok(Some(bytes))) => Some(std::sync::Arc::new(
                    crate::history::walk_segment_records(&bytes)?,
                )),
                Some(Ok(None)) | None => None,
                Some(Err(e)) => return Err(e),
            };
            let records = match records {
                Some(records) => Some(records),
                // Local copy absent: consult the remote mirror. The P2.7
                // sidecar object (KBs) prunes the segment fetch (100s of
                // KB) when the segment provably cannot decide this key —
                // any sidecar problem (absent = pre-P2.7 upload or
                // pre-P2.6 segment, unreadable, corrupt) fails open to
                // the fetch+walk, exactly like the local sidecar.
                None => {
                    let Some(remote) = self.remote_history.as_ref() else {
                        missing_below_snap = true;
                        continue;
                    };
                    if let Ok(Some(buf)) = remote.tier.read_sidecar(&remote.env, name) {
                        if !crate::history::HistoryTier::sidecar_may_affect(&buf, key) {
                            continue; // sound skip — spans below still count it
                        }
                    }
                    // P2.8 read cache: a hit skips the fetch and the walk
                    // (entries are CRC-verified on insert and the name is
                    // content-addressed — stable for the bytes). The
                    // guard is dropped at the `let` so the miss path can
                    // re-lock for the insert.
                    let cached = self.remote_read_cache.lock().get(name);
                    if let Some(records) = cached {
                        Some(records)
                    } else {
                        // A missing object is a coverage gap (fail-closed
                        // below); anything else (corrupt read-back)
                        // propagates typed.
                        match remote.tier.read_segment(&remote.env, name) {
                            Ok(bytes) => {
                                let records = std::sync::Arc::new(
                                    crate::history::walk_segment_records(&bytes)?,
                                );
                                let cost = bytes.len() as u64;
                                self.remote_read_cache
                                    .lock()
                                    .insert(name, records.clone(), cost);
                                Some(records)
                            }
                            Err(CoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                                missing_below_snap = true;
                                None
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
            };
            let Some(records) = records else { continue };
            if let Some(rec) = crate::history::decide_at(&records, key, snap.seq) {
                if best.as_ref().map_or(true, |b| rec.seq > b.seq) {
                    best = Some(rec.clone());
                }
            }
        }
        if let Some(rec) = best {
            return Ok(match rec.kind {
                0 => Some(Bytes::from(rec.val)),
                _ => None, // delete / range delete covering the key
            });
        }
        // No deciding record. `None` is provable only when the retained
        // segments cover [1, snap] contiguously and the cap never dropped
        // anything at/below snap (archive_floor is the drop high-water).
        if missing_below_snap || tier.archive_floor() > snap.seq {
            return Err(too_old());
        }
        let mut spans: Vec<(u64, u64)> = cands.iter().map(|(f, t, _, _, _)| (*f, *t)).collect();
        spans.sort_unstable();
        let mut covered_to = 1u64;
        for (from, through) in spans {
            if from > covered_to {
                return Err(too_old()); // gap below snap
            }
            covered_to = covered_to.max(through + 1);
            if covered_to > snap.seq {
                break;
            }
        }
        if covered_to > snap.seq {
            Ok(None)
        } else {
            Err(too_old())
        }
    }

    /// RFC-0046 P2.3: LSM leg of a below-watermark read whose archive
    /// coverage failed. Soundness: a found version (put or tombstone) at
    /// `seq ≤ snap` is physically present — serve it. Anything else keeps
    /// the tier's `SnapshotTooOld`: the LSM cannot prove a key was never
    /// written (all its versions may have been GC'd and tombstone-cleaned),
    /// so `None` here would be a silent destroy.
    pub(super) fn get_at_below_watermark_lsm(&self, snap: Snapshot, key: &[u8]) -> Result<Option<Bytes>> {
        let too_old = || CoreError::snapshot_too_old(snap.seq, self.earliest_readable_seq);
        match self.lookup(key, snap.seq) {
            Lookup::Found(v) => {
                if vlog::decode_vlog_ptr(v.as_ref()).is_some() {
                    self.get_vlog.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.get_inline.fetch_add(1, Ordering::Relaxed);
                }
                // F1: corruption surfaces as Err, never as a miss.
                Ok(Some(self.resolve_stored_value(v)?))
            }
            Lookup::Deleted => Ok(None),
            Lookup::NotFound => Err(too_old()),
        }
    }

}
