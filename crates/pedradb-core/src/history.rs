//! RFC-0046 P0.2: bounded local history tier — archive before version GC.
//!
//! Before auto-compact GC drops superseded versions, they are appended to
//! CRC'd history segments (`history/seg-*.hist`, streamed in bounded chunks).
//! `history/MANIFEST` is the truth: it references a segment only after
//! `sync_all`, so a crash mid-archive leaves at most an unreferenced file
//! (removed on next open). On cap overflow the oldest segments are dropped
//! and the archive floor advances — snaps below it are
//! `CoreError::SnapshotTooOld` (typed, never a silent destroy). An open pin
//! holds every segment at/below it.

use crate::env::{Env, EnvFile};
use crate::error::{CoreError, Result};
use crate::wal::crc::crc32c;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// One archived segment (manifest entry; file at `history/seg-{id:08}.hist`,
/// mirrored remotely under its content-addressed `name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SegmentMeta {
    pub id: u64,
    /// Content-addressed remote object name (`seg-<len>-<crc32c>.hist`).
    pub name: String,
    pub from_seq: u64,
    pub through_seq: u64,
    pub bytes: u64,
    /// Key-coverage floor (P2.5 pruning; `None` on v2 manifests sealed
    /// before the field existed — those segments always walk).
    pub key_lo: Option<Vec<u8>>,
    /// Key-coverage ceiling, **range-delete aware**: the exclusive end of
    /// any range delete counts, so a record affecting `key ∈ [lo, hi]`
    /// cannot exist outside the bound (sound skip for reads).
    pub key_hi: Option<Vec<u8>>,
}

/// Max records per segment (bounds memory nothing — records stream; keeps
/// segments small enough to rotate the cap cheaply).
const SEG_MAX_RECORDS: usize = 8192;

/// History-tier manifest state (persisted atomically via tmp+rename).
#[derive(Debug, Default)]
struct Manifest {
    next_id: u64,
    /// Segments oldest-first (creation order).
    segs: VecDeque<SegmentMeta>,
    /// Highest seq whose history was dropped for cap (GC floor input).
    archive_floor: u64,
}

impl Manifest {
    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(28 + self.segs.len() * 40);
        b.extend_from_slice(b"PHST");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&self.next_id.to_le_bytes());
        b.extend_from_slice(&self.archive_floor.to_le_bytes());
        b.extend_from_slice(&(self.segs.len() as u32).to_le_bytes());
        for s in &self.segs {
            b.extend_from_slice(&s.id.to_le_bytes());
            b.extend_from_slice(&(s.name.len() as u32).to_le_bytes());
            b.extend_from_slice(s.name.as_bytes());
            b.extend_from_slice(&s.from_seq.to_le_bytes());
            b.extend_from_slice(&s.through_seq.to_le_bytes());
            b.extend_from_slice(&s.bytes.to_le_bytes());
            // P2.5 key coverage: present flag + bytes (v2 manifests decode
            // with None — those segments always walk).
            let keyed = s.key_lo.is_some() && s.key_hi.is_some();
            b.push(u8::from(keyed));
            if keyed {
                let lo = s.key_lo.as_ref().expect("checked");
                let hi = s.key_hi.as_ref().expect("checked");
                b.extend_from_slice(&(lo.len() as u32).to_le_bytes());
                b.extend_from_slice(lo);
                b.extend_from_slice(&(hi.len() as u32).to_le_bytes());
                b.extend_from_slice(hi);
            }
        }
        let crc = crc32c(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    /// RFC-0082 P1.2 / RFC-0086 P0: trailer CRC is `crc_match_ok`.
    fn decode(buf: &[u8]) -> Result<Self> {
        let bad = || CoreError::CorruptManifest("history manifest".into());
        if buf.len() < 32 || &buf[0..4] != b"PHST" {
            return Err(bad());
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        if version != 2 && version != 3 {
            return Err(bad());
        }
        let body_len = buf.len() - 4;
        let crc = u32::from_le_bytes(buf[body_len..].try_into().unwrap());
        if !crate::wal::crc::crc_match_ok(crc32c(&buf[..body_len]), crc) {
            return Err(CoreError::CorruptManifest(
                "history manifest crc mismatch".into(),
            ));
        }
        let next_id = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let archive_floor = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let n = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
        // F199: `n` is untrusted (remote manifests decode through here).
        // Each entry consumes at least 36 body bytes (v2: id + name len +
        // from/through/bytes; v3 adds the keyed flag), so a count the body
        // cannot possibly hold is a corrupt/attack manifest — reject it
        // BEFORE the `with_capacity` allocation: n = u32::MAX otherwise
        // reserves ~446 GB (`n × sizeof(SegmentMeta)`) and strict-overcommit
        // hosts abort the process on a 36-byte remote object.
        const MIN_ENTRY_BYTES: usize = 36;
        if n > (body_len - 28) / MIN_ENTRY_BYTES {
            return Err(bad());
        }
        let mut segs = VecDeque::with_capacity(n);
        let mut off = 28;
        for _ in 0..n {
            let g = |off: &mut usize| -> Result<u64> {
                if *off + 8 > body_len {
                    return Err(bad());
                }
                let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
                *off += 8;
                Ok(v)
            };
            let id = g(&mut off)?;
            if off + 4 > body_len {
                return Err(bad());
            }
            let nlen = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            if off + nlen + 24 > body_len {
                return Err(bad());
            }
            let name = String::from_utf8_lossy(&buf[off..off + nlen]).into_owned();
            off += nlen;
            let from_seq = g(&mut off)?;
            let through_seq = g(&mut off)?;
            let bytes = g(&mut off)?;
            let (key_lo, key_hi) = if version >= 3 {
                if off >= body_len {
                    return Err(bad());
                }
                let keyed = buf[off] == 1;
                off += 1;
                if !keyed {
                    (None, None)
                } else {
                    let take = |off: &mut usize| -> Result<Vec<u8>> {
                        if *off + 4 > body_len {
                            return Err(bad());
                        }
                        let l =
                            u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap()) as usize;
                        *off += 4;
                        if *off + l > body_len {
                            return Err(bad());
                        }
                        let v = buf[*off..*off + l].to_vec();
                        *off += l;
                        Ok(v)
                    };
                    (Some(take(&mut off)?), Some(take(&mut off)?))
                }
            } else {
                (None, None)
            };
            segs.push_back(SegmentMeta {
                id,
                name,
                from_seq,
                through_seq,
                bytes,
                key_lo,
                key_hi,
            });
        }
        Ok(Self {
            next_id,
            segs,
            archive_floor,
        })
    }
}

/// The local history tier of one database directory.
#[derive(Debug)]
pub(crate) struct HistoryTier {
    root: PathBuf,
    manifest: Manifest,
}

impl HistoryTier {
    /// Open the tier under `db_root/history`. Nothing is created until the
    /// first archived record (a read-only open of a fresh DB leaves the
    /// directory flat). Unreferenced segment files (crash leftovers) are
    /// removed — the manifest is the truth.
    pub(crate) fn open<E: Env>(env: &E, db_root: &Path) -> Result<Self> {
        let dir = db_root.join("history");
        let mut manifest = Manifest {
            next_id: 1,
            segs: VecDeque::new(),
            archive_floor: 0,
        };
        if env.exists(&dir) {
            let path = dir.join("MANIFEST");
            if env.exists(&path) {
                let mut f = env.open_read(&path)?;
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut buf)?;
                manifest = Manifest::decode(&buf)?;
            }
            let live: std::collections::HashSet<u64> = manifest.segs.iter().map(|s| s.id).collect();
            for name in env.read_dir_names(&dir).unwrap_or_default() {
                // Both segment data (.hist) and P2.6 bloom sidecars (.bloom)
                // without a manifest entry are crash leftovers (manifest is
                // the truth).
                let stem = name
                    .strip_suffix(".hist")
                    .or_else(|| name.strip_suffix(".bloom"));
                if let Some(id) = stem
                    .and_then(|s| s.strip_prefix("seg-"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    if !live.contains(&id) {
                        let _ = env.remove_file(&dir.join(name));
                    }
                }
            }
        }
        Ok(Self {
            root: db_root.to_path_buf(),
            manifest,
        })
    }

    /// Stream `records` (all already filtered to what GC will drop) into
    /// synced segments, chunked at [`SEG_MAX_RECORDS`]. Each completed chunk
    /// is fsynced and the manifest atomically updated before this returns.
    pub(crate) fn archive_stream<E, I>(&mut self, env: &E, records: I) -> Result<()>
    where
        E: Env,
        I: Iterator<Item = (Vec<u8>, Vec<u8>, u64, u8)>,
    {
        let dir = self.root.join("history");
        let mut cur_file = None;
        let mut cur_id = 0u64;
        let mut cur_n = 0usize;
        let mut cur_bytes = 0u64;
        let mut cur_from = u64::MAX;
        let mut cur_through = 0u64;
        // P2.5 key coverage: floor over record keys, ceiling over record
        // keys AND range-delete ends (kind 2 stores the exclusive end in
        // `val`) — any record affecting a key inside [lo, hi] stays inside.
        let mut cur_key_lo: Vec<u8> = Vec::new();
        let mut cur_key_hi: Vec<u8> = Vec::new();
        let mut key_lo_set = false;
        // P2.6 bloom sidecar: point-record keys + range-delete intervals,
        // collected per segment and written at seal. Point keys feed the
        // bloom; range deletes cannot (an interval is not enumerable), so
        // they ride as explicit intervals — pruning stays sound.
        let mut cur_point_keys: Vec<Vec<u8>> = Vec::new();
        let mut cur_range_dels: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut buf: Vec<u8> = Vec::new();
        for (key, val, seq, kind) in records {
            if cur_file.is_none() {
                env.create_dir_all(&dir)?;
                cur_id = self.manifest.next_id;
                self.manifest.next_id += 1;
                cur_file = Some(env.create(&dir.join(format!("seg-{cur_id:08}.hist")))?);
                (cur_n, cur_bytes, cur_from, cur_through) = (0, 0, u64::MAX, 0);
                cur_key_lo.clear();
                cur_key_hi.clear();
                key_lo_set = false;
                cur_point_keys.clear();
                cur_range_dels.clear();
            }
            buf.clear();
            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            buf.extend_from_slice(&key);
            buf.extend_from_slice(&(val.len() as u32).to_le_bytes());
            buf.extend_from_slice(&val);
            buf.extend_from_slice(&seq.to_le_bytes());
            buf.push(kind);
            buf.extend_from_slice(&crc32c(&buf).to_le_bytes());
            cur_file.as_mut().unwrap().write_all(&buf)?;
            cur_n += 1;
            cur_bytes += buf.len() as u64;
            cur_from = cur_from.min(seq);
            cur_through = cur_through.max(seq);
            if !key_lo_set || key.as_slice() < cur_key_lo.as_slice() {
                cur_key_lo.clear();
                cur_key_lo.extend_from_slice(&key);
                key_lo_set = true;
            }
            let ceiling = if kind == 2 {
                val.as_slice()
            } else {
                key.as_slice()
            };
            if ceiling > cur_key_hi.as_slice() {
                cur_key_hi.clear();
                cur_key_hi.extend_from_slice(ceiling);
            }
            if kind == 2 {
                cur_range_dels.push((key, val));
            } else {
                cur_point_keys.push(key);
            }
            if cur_n >= SEG_MAX_RECORDS {
                self.seal_segment(
                    env,
                    cur_file.take().unwrap(),
                    cur_id,
                    cur_from,
                    cur_through,
                    cur_bytes,
                    Some((cur_key_lo.clone(), cur_key_hi.clone())),
                    std::mem::take(&mut cur_point_keys),
                    std::mem::take(&mut cur_range_dels),
                )?;
            }
        }
        if let Some(f) = cur_file.take() {
            self.seal_segment(
                env,
                f,
                cur_id,
                cur_from,
                cur_through,
                cur_bytes,
                Some((cur_key_lo, cur_key_hi)),
                cur_point_keys,
                cur_range_dels,
            )?;
        }
        Ok(())
    }

    fn seal_segment<E: Env>(
        &mut self,
        env: &E,
        mut f: E::File,
        id: u64,
        from: u64,
        through: u64,
        bytes: u64,
        key_coverage: Option<(Vec<u8>, Vec<u8>)>,
        point_keys: Vec<Vec<u8>>,
        range_dels: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<()> {
        f.sync_all()?;
        drop(f);
        // P2.6: bloom sidecar — synced BEFORE the manifest persists, so a
        // manifest-referenced segment always has its sidecar durable (a
        // crash between the two leaves an orphan sidecar, removed at open).
        self.write_bloom_sidecar(env, id, &point_keys, &range_dels)?;
        // Content-addressed name (remote mirror object identity): read the
        // sealed bytes back once and hash them.
        let path = self.root.join("history").join(format!("seg-{id:08}.hist"));
        let mut rf = env.open_read(&path)?;
        let mut buf = Vec::with_capacity(bytes as usize);
        std::io::Read::read_to_end(&mut rf, &mut buf)?;
        let name = RemoteTier::segment_name(&buf);
        let (key_lo, key_hi) = match key_coverage {
            // An empty segment carries no coverage — keep `None` (always
            // walks; decode of an all-empty tier stays `None` too).
            Some((lo, hi)) if !lo.is_empty() || !hi.is_empty() => (Some(lo), Some(hi)),
            _ => (None, None),
        };
        self.manifest.segs.push_back(SegmentMeta {
            id,
            name,
            from_seq: from,
            through_seq: through,
            bytes,
            key_lo,
            key_hi,
        });
        self.persist(env)
    }

    /// P2.6: write `history/seg-{id:08}.bloom`. Layout: magic `PHB1` +
    /// version u32 | body | footer (body_len u64 + crc32c(body) u32).
    /// Body: `BloomFilter::encode()` (self-framed) + rd_count u32 +
    /// intervals (len-prefixed start/end). Covers point keys by bloom and
    /// range deletes explicitly — a key is provably unaffected only when
    /// the bloom says absent AND no interval covers it.
    fn write_bloom_sidecar<E: Env>(
        &self,
        env: &E,
        id: u64,
        point_keys: &[Vec<u8>],
        range_dels: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<()> {
        let mut bloom = crate::bloom::BloomFilter::with_capacity(
            point_keys.len(),
            crate::bloom::DEFAULT_BITS_PER_KEY,
        );
        for k in point_keys {
            bloom.insert(k);
        }
        let mut body = bloom.encode();
        body.extend_from_slice(&(range_dels.len() as u32).to_le_bytes());
        for (start, end) in range_dels {
            body.extend_from_slice(&(start.len() as u32).to_le_bytes());
            body.extend_from_slice(start);
            body.extend_from_slice(&(end.len() as u32).to_le_bytes());
            body.extend_from_slice(end);
        }
        let path = self.root.join("history").join(format!("seg-{id:08}.bloom"));
        let mut out = Vec::with_capacity(8 + body.len() + 12);
        out.extend_from_slice(b"PHB1");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&crc32c(&body).to_le_bytes());
        let mut f = env.create(&path)?;
        f.write_all(&out)?;
        f.sync_all()?;
        Ok(())
    }

    /// P2.6: `false` ⇒ the segment provably cannot decide `key` (bloom
    /// negative and no range-delete interval covers it) — safe to skip the
    /// record walk. Anything else — missing sidecar (pre-P2.6 segment),
    /// corrupt sidecar, I/O error — returns `true` (walk): a damaged
    /// filter must never prune. `Err` propagates only from unexpected
    /// read failures, and even then the caller treats it as "walk".
    pub(crate) fn segment_may_affect<E: Env>(&self, env: &E, id: u64, key: &[u8]) -> bool {
        let path = self.root.join("history").join(format!("seg-{id:08}.bloom"));
        let Ok(mut f) = env.open_read(&path) else {
            return true; // no sidecar — cannot prune
        };
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return true;
        }
        Self::sidecar_may_affect(&buf, key)
    }

    /// Pure parse of a bloom sidecar (test surface). Fails open to `true`.
    /// RFC-0088 P1.1: trailer CRC is `crc_match_ok`. Mismatch returns true
    /// (walk, never prune). Scrub (`verify_bloom_sidecar`) is fail-closed;
    /// this path is not.
    pub(crate) fn sidecar_may_affect(buf: &[u8], key: &[u8]) -> bool {
        let footer_len = 12usize;
        if buf.len() < 8 + footer_len || &buf[0..4] != b"PHB1" {
            return true;
        }
        if u32::from_le_bytes(buf[4..8].try_into().unwrap()) != 1 {
            return true;
        }
        let body_len = u64::from_le_bytes(
            buf[buf.len() - footer_len..buf.len() - 4]
                .try_into()
                .unwrap(),
        ) as usize;
        if body_len + 8 + footer_len != buf.len() {
            return true;
        }
        let body = &buf[8..8 + body_len];
        let crc = u32::from_le_bytes(buf[buf.len() - 4..].try_into().unwrap());
        if !crate::wal::crc::crc_match_ok(crc, crc32c(body)) {
            return true; // corrupt sidecar — never prune
        }
        // Bloom blob is self-framed: nbits/k/nbytes then bits.
        if body.len() < 12 {
            return true;
        }
        let nbytes = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
        let bloom_end = 12 + nbytes;
        if bloom_end + 4 > body.len() {
            return true;
        }
        let bloom = match crate::bloom::BloomFilter::decode(&body[..bloom_end]) {
            Ok(b) => b,
            Err(_) => return true,
        };
        if bloom.may_contain(key) {
            return true;
        }
        let mut off = bloom_end;
        let rd_count = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()) as usize;
        off += 4;
        for _ in 0..rd_count {
            if off + 4 > body.len() {
                return true; // truncated interval list — fail open
            }
            let sl = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            if off + sl + 4 > body.len() {
                return true;
            }
            let start = &body[off..off + sl];
            off += sl;
            let el = u32::from_le_bytes(body[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            if off + el > body.len() {
                return true;
            }
            let end = &body[off..off + el];
            off += el;
            // Range delete hides [start, end): affects keys in it.
            if key >= start && key < end {
                return true;
            }
        }
        false
    }

    fn persist<E: Env>(&mut self, env: &E) -> Result<()> {
        let dir = self.root.join("history");
        env.create_dir_all(&dir)?;
        let tmp = dir.join("MANIFEST.tmp");
        let fin = dir.join("MANIFEST");
        {
            let mut f = env.create(&tmp)?;
            f.write_all(&self.manifest.encode())?;
            f.sync_all()?;
        }
        env.rename(&tmp, &fin)?;
        let _ = env.sync_dir(&dir);
        Ok(())
    }

    /// Drop oldest segments above `cap_bytes` (0 = unbounded). An open pin
    /// (`pin_floor` = oldest pinned seq) holds every segment at/below it.
    /// `hold` (P1.2, remote tier configured) additionally holds every
    /// segment whose id is NOT in `uploaded` — backpressure: the cap is a
    /// soft target while the remote tier is down; local disk grows rather
    /// than destroy history that never uploaded. `None` = local-only P0
    /// semantics (cap drops freely, watermark advances typed).
    /// Returns the archive floor after enforcement (monotonic).
    pub(crate) fn enforce_cap<E: Env>(
        &mut self,
        env: &E,
        pin_floor: Option<u64>,
        cap_bytes: u64,
        uploaded: Option<&std::collections::HashSet<u64>>,
    ) -> Result<u64> {
        if cap_bytes == 0 {
            return Ok(self.manifest.archive_floor);
        }
        let mut total: u64 = self.manifest.segs.iter().map(|s| s.bytes).sum();
        let dir = self.root.join("history");
        while total > cap_bytes {
            let Some(front) = self.manifest.segs.front().cloned() else {
                break;
            };
            // A pin at/below the segment's top holds it — stop (fail-closed
            // toward keeping history, never toward dropping pinned data).
            if let Some(pin) = pin_floor {
                if pin <= front.through_seq {
                    break;
                }
            }
            if let Some(uploaded) = uploaded {
                if !uploaded.contains(&front.id) {
                    break; // not verified at the remote tier — keep it
                }
            }
            let _ = env.remove_file(&dir.join(format!("seg-{:08}.hist", front.id)));
            let _ = env.remove_file(&dir.join(format!("seg-{:08}.bloom", front.id)));
            total -= front.bytes;
            self.manifest.archive_floor = self.manifest.archive_floor.max(front.through_seq + 1);
            self.manifest.segs.pop_front();
        }
        self.persist(env)?;
        Ok(self.manifest.archive_floor)
    }

    /// Highest seq dropped for cap (0 = nothing dropped).
    pub(crate) fn archive_floor(&self) -> u64 {
        self.manifest.archive_floor
    }

    /// Live archived bytes.
    pub(crate) fn bytes(&self) -> u64 {
        self.manifest.segs.iter().map(|s| s.bytes).sum()
    }

    /// Manifest entries of live local segments, oldest-first (P2.1 lazy
    /// read + P2.2 metrics input).
    pub(crate) fn segment_metas(&self) -> Vec<SegmentMeta> {
        self.manifest.segs.iter().cloned().collect()
    }

    /// Bytes of one local segment file, `None` when absent (cap-dropped or
    /// never sealed). Corrupt-but-present is the caller's CRC walk to catch.
    pub(crate) fn read_local_segment<E: Env>(&self, env: &E, id: u64) -> Result<Option<Vec<u8>>> {
        let path = Self::segment_path(&self.root, id);
        match env.open_read(&path) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                Ok(Some(buf))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Encode the manifest (P1.1: the remote tier uploads these bytes as an
    /// immutable generation object).
    pub(crate) fn manifest_bytes(&self) -> Vec<u8> {
        self.manifest.encode()
    }

    /// Next immutable manifest generation id for the remote tier.
    pub(crate) fn remote_generation(&self) -> u64 {
        self.manifest.next_id
    }

    /// Ids of live local segments, oldest-first (P1.2 upload pass input).
    pub(crate) fn segment_ids(&self) -> Vec<u64> {
        self.manifest.segs.iter().map(|s| s.id).collect()
    }

    /// Path of one local segment file.
    pub(crate) fn segment_path(db_root: &Path, id: u64) -> PathBuf {
        db_root.join("history").join(format!("seg-{id:08}.hist"))
    }
}

/// Outcome of uploading one object to the remote tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutStatus {
    /// Object written and synced at the destination.
    Uploaded,
    /// Identical content already present (read-back verified).
    AlreadyPresent,
}

/// Report of one remote upload pass (RFC-0046 P1.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadReport {
    /// Segments written at the destination this pass.
    pub segments_uploaded: usize,
    /// Segments already present (read-back verified) — resume is free.
    pub segments_already_present: usize,
    /// Manifest generation upload outcome (`None` = nothing to ship).
    pub manifest: Option<PutStatus>,
}

/// One record as stored in a history segment (wire form: `u32 klen, key,
/// u32 vlen, val, u64 seq, u8 kind, u32 crc32c` over everything before it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRecord {
    /// User key bytes.
    pub key: Vec<u8>,
    /// Stored value bytes (deletes carry an empty payload; the kind
    /// discriminates).
    pub val: Vec<u8>,
    /// Publish sequence of the version.
    pub seq: u64,
    /// 0 = value, 1 = delete, 2 = range delete.
    pub kind: u8,
}

/// Walk every record of a serialized segment, verifying the per-record CRC.
/// Returns the records; corrupt or truncated input is a typed error
/// (fail-closed — used both before upload and at restore time).
/// RFC-0087 P0: per-record CRC is `crc_match_ok`. P2.2: upload
/// (`put_segment`) and restore/scrub callers (db.rs / ops / verify.rs)
/// stay on this walker.
pub fn walk_segment_records(bytes: &[u8]) -> Result<Vec<HistoryRecord>> {
    let bad = |why: &str| CoreError::CorruptHistory(format!("segment record {why}"));
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < bytes.len() {
        let start = off;
        let rd_u32 = |off: &mut usize| -> Result<u32> {
            if *off + 4 > bytes.len() {
                return Err(bad("truncated header"));
            }
            let v = u32::from_le_bytes(bytes[*off..*off + 4].try_into().unwrap());
            *off += 4;
            Ok(v)
        };
        let klen = rd_u32(&mut off)? as usize;
        if off + klen > bytes.len() {
            return Err(bad("truncated key"));
        }
        let key = bytes[off..off + klen].to_vec();
        off += klen;
        let vlen = rd_u32(&mut off)? as usize;
        if off + vlen > bytes.len() {
            return Err(bad("truncated value"));
        }
        let val = bytes[off..off + vlen].to_vec();
        off += vlen;
        if off + 8 + 1 + 4 > bytes.len() {
            return Err(bad("truncated tail"));
        }
        let seq = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
        off += 8;
        let kind = bytes[off];
        off += 1;
        let stored = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        off += 4;
        if !crate::wal::crc::crc_match_ok(crc32c(&bytes[start..off - 4]), stored) {
            return Err(bad("crc mismatch"));
        }
        out.push(HistoryRecord {
            key,
            val,
            seq,
            kind,
        });
    }
    Ok(out)
}

/// RFC-0060: decode + CRC the history-tier `history/MANIFEST` (`PHST` trailer).
///
/// # Errors
/// Corrupt magic, version, length, or CRC.
pub fn verify_history_manifest(bytes: &[u8]) -> Result<()> {
    Manifest::decode(bytes).map(|_| ())
}

/// RFC-0060 / RFC-0088: bloom sidecar CRC fail-closed (scrub is not the
/// read-path fail-open used when deciding whether to skip a segment).
///
/// # Errors
/// Bad magic/version/length, or CRC mismatch (`crc_match_ok`).
pub fn verify_bloom_sidecar(bytes: &[u8]) -> Result<()> {
    let bad = || CoreError::CorruptHistory("bloom sidecar".into());
    const FOOTER: usize = 12;
    if bytes.len() < 8 + FOOTER || &bytes[0..4] != b"PHB1" {
        return Err(bad());
    }
    if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 1 {
        return Err(bad());
    }
    let body_len = u64::from_le_bytes(
        bytes[bytes.len() - FOOTER..bytes.len() - 4]
            .try_into()
            .unwrap(),
    ) as usize;
    if body_len.saturating_add(8).saturating_add(FOOTER) != bytes.len() {
        return Err(bad());
    }
    let body = &bytes[8..8 + body_len];
    let crc = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap());
    if !crate::wal::crc::crc_match_ok(crc32c(body), crc) {
        return Err(CoreError::CorruptHistory(
            "bloom sidecar crc mismatch".into(),
        ));
    }
    Ok(())
}

/// RFC-0046 P2.1: newest record that decides `key`'s visibility at `seq` —
/// exact puts/deletes (`kind` 0/1, `record.key == key`) and range deletes
/// (`kind` 2, `key` stored as the range start with the end in `val`).
/// Records arrive in archive order, not seq order — the max-seq match wins.
pub(crate) fn decide_at<'a>(
    records: &'a [HistoryRecord],
    key: &[u8],
    seq: u64,
) -> Option<&'a HistoryRecord> {
    records
        .iter()
        .filter(|r| r.seq <= seq)
        .filter(|r| match r.kind {
            2 => r.key.as_slice() <= key && key < r.val.as_slice(),
            _ => r.key == key,
        })
        .max_by_key(|r| r.seq)
}

/// RFC-0046 P2.2: history-tier roll-up (local + remote + upload backlog).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryStats {
    /// Segments live in the local tier.
    pub local_segments: usize,
    /// Bytes live in the local tier.
    pub local_bytes: u64,
    /// Highest seq the local cap already dropped (0 = nothing dropped).
    pub archive_floor: u64,
    /// Lowest sequence still guaranteed readable in-DB (the GC watermark;
    /// below it reads go through the tier, P2.1).
    pub earliest_readable: u64,
    /// Local segments not yet verified at the remote tier (0 when no
    /// remote is configured). Non-zero means the cap is holding them
    /// (backpressure — local disk grows until uploads catch up).
    pub pending_uploads: usize,
    /// Remote mirror roll-up (`None` = no remote configured or nothing
    /// shipped yet). Reading it touches the destination; errors propagate
    /// (fail-closed).
    pub remote: Option<RemoteSummary>,
    /// Milliseconds since the last archive pass this open (`None` until
    /// the first one — in-memory, resets on reopen).
    pub last_archive_age_millis: Option<u64>,
    /// Horizon `(seq, time)` samples currently held (RFC-0046 P0.1).
    /// Hard-bounded by `HORIZON_SAMPLE_RING_CAP` — long windows under
    /// sustained writes must not grow memory without bound.
    pub seq_time_samples: usize,
    /// RFC-0046 P2.8 remote read cache: verified-decoded segments held
    /// in memory (0 when no remote is configured or the budget is 0).
    pub remote_cache_entries: usize,
    /// RFC-0046 P2.8 remote read cache: bytes held (bounded by the
    /// configured budget).
    pub remote_cache_bytes: u64,
}

/// One segment as listed by the remote manifest (restore input,
/// RFC-0046 P1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSegment {
    /// Content-addressed object name under the tier root.
    pub name: String,
    /// Lowest publish seq covered.
    pub from_seq: u64,
    /// Highest publish seq covered.
    pub through_seq: u64,
    /// Segment size in bytes.
    pub bytes: u64,
    /// Lowest key the segment's manifest bound covers (`None` = pre-P2.5
    /// remote manifest — the reader walks; the bound is advisory for
    /// pruning only, RFC-0046 P2.7).
    pub key_lo: Option<Vec<u8>>,
    /// Highest key the segment's manifest bound covers (inclusive).
    pub key_hi: Option<Vec<u8>>,
}

/// Roll-up of the newest intact remote manifest (status output,
/// RFC-0046 P1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteSummary {
    /// Live segment objects listed by the manifest.
    pub segments: usize,
    /// Total archived bytes.
    pub bytes: u64,
    /// Lowest seq covered (0 when empty).
    pub from_seq: u64,
    /// Highest seq covered (0 when empty).
    pub through_seq: u64,
    /// Highest seq whose local history was already dropped for cap.
    pub archive_floor: u64,
    /// Next immutable manifest generation id.
    pub next_generation: u64,
}

/// RFC-0060 P2.11: at-rest CRC walk of a remote history tier.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteVerifyReport {
    /// Segments listed by the newest intact manifest.
    pub segments: usize,
    /// Bloom sidecars found next to those segments.
    pub sidecars: usize,
    /// CRC/decode failures.
    pub errors: u64,
    /// `(object name, message)` for each failure.
    pub failures: Vec<(String, String)>,
}

impl RemoteVerifyReport {
    /// Product stdout line.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "segments={} sidecars={} errors={}",
            self.segments, self.sidecars, self.errors
        )
    }

    /// True when every listed object decoded.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.errors == 0
    }
}

/// RFC-0046 P1.1: object-storage-shaped mirror of the local history tier,
/// reached only through the `Env` seam (no network in unit tests — the
/// destination is any `Env`; an S3-class binding is a host-side `Env` impl).
///
/// Layout under `root`:
/// - `seg-<len:016x>-<crc32c:08x>.hist` — immutable, content-addressed
///   segment objects. Dedup is **read-back verified**: a name hit with
///   different bytes is a typed collision error, never silent wrong data.
/// - `MANIFEST-<n:016>` — immutable manifest generations (`n` = local
///   `next_id`).
/// - `LATEST` — tiny pointer (`MANIFEST-<n:016>\n<crc32c of that manifest>`)
///   rewritten per upload. Object stores have no rename; a torn `LATEST`
///   makes the reader fall back to the newest intact generation.
#[derive(Debug, Clone)]
pub struct RemoteTier {
    root: PathBuf,
}

impl RemoteTier {
    /// Remote tier rooted at `root` (created lazily on first upload).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Content-addressed object name for segment `bytes`. Two digests:
    /// crc32c alone collided on a structured workload (same key set with
    /// shifted seqs — 3-wave overwrite test, 2026-08-21); FNV-1a 64 is
    /// the independent second check. The read-back byte verify in
    /// `put_segment` stays regardless.
    pub fn segment_name(bytes: &[u8]) -> String {
        format!(
            "seg-{:016x}-{:08x}-{:016x}.hist",
            bytes.len() as u64,
            crc32c(bytes),
            crate::bloom::fnv1a64_pub(bytes)
        )
    }

    fn segment_path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Upload one sealed local segment. The bytes are walked and CRC-verified
    /// **before** anything leaves the machine (corrupt history never
    /// uploads), then written `create + write_all + sync_all` with a synced
    /// directory. Idempotent: identical content already present is a
    /// read-back-verified no-op (P1.2 retry/resume builds on this).
    pub fn put_segment<R: Env, L: Env>(
        &self,
        remote_env: &R,
        local_env: &L,
        local_path: &Path,
    ) -> Result<PutStatus> {
        let mut f = local_env.open_read(local_path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes)?;
        walk_segment_records(&bytes)?;
        let name = Self::segment_name(&bytes);
        let dest = self.segment_path(&name);
        let status = if remote_env.exists(&dest) {
            let mut rf = remote_env.open_read(&dest)?;
            let mut have = Vec::new();
            std::io::Read::read_to_end(&mut rf, &mut have)?;
            if have.len() != bytes.len() {
                return Err(CoreError::CorruptHistory(format!(
                    "remote name collision at {name}: read-back differs"
                )));
            }
            if !crate::wal::crc::crc_match_ok(crc32c(&have), crc32c(&bytes)) {
                return Err(CoreError::CorruptHistory(format!(
                    "remote name collision at {name}: crc mismatch"
                )));
            }
            // RFC-0092 P2.1: CRC match is not a collision theorem (R-crc).
            // Byte-equal after the CRC gate; checking bytes first would
            // hide the P0 same-length XOR tooth.
            if have != bytes {
                return Err(CoreError::CorruptHistory(format!(
                    "remote name collision at {name}: read-back differs"
                )));
            }
            PutStatus::AlreadyPresent
        } else {
            remote_env.create_dir_all(&self.root)?;
            {
                let mut out = remote_env.create(&dest)?;
                out.write_all(&bytes)?;
                out.sync_all()?;
            }
            remote_env.sync_dir(&self.root)?;
            PutStatus::Uploaded
        };
        // RFC-0046 P2.7: ship the bloom sidecar next to the object
        // (`<segment-name>.bloom`) in BOTH branches — retry/resume covers
        // a sidecar the previous pass never got to. Segment first, then
        // sidecar, then (caller) manifest: a listed segment may lack its
        // sidecar only in the harmless direction — the reader walks.
        // A segment without a local sidecar (pre-P2.6) ships nothing.
        let sidecar = local_path.with_extension("bloom");
        if local_env.exists(&sidecar) {
            self.put_sidecar(remote_env, local_env, &sidecar, &name)?;
        }
        Ok(status)
    }

    /// Upload one sidecar under `<remote_name>.bloom`, idempotent the same
    /// way as [`Self::put_segment`] (present + crc-identical = no-op;
    /// a differing read-back is a collision and fails closed).
    fn put_sidecar<R: Env, L: Env>(
        &self,
        remote_env: &R,
        local_env: &L,
        local_path: &Path,
        remote_name: &str,
    ) -> Result<()> {
        let mut f = local_env.open_read(local_path)?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes)?;
        let dest = self.segment_path(&format!("{remote_name}.bloom"));
        if remote_env.exists(&dest) {
            let mut rf = remote_env.open_read(&dest)?;
            let mut have = Vec::new();
            std::io::Read::read_to_end(&mut rf, &mut have)?;
            if have.len() != bytes.len() {
                return Err(CoreError::CorruptHistory(format!(
                    "remote sidecar collision at {remote_name}.bloom: read-back differs"
                )));
            }
            if !crate::wal::crc::crc_match_ok(crc32c(&have), crc32c(&bytes)) {
                return Err(CoreError::CorruptHistory(format!(
                    "remote sidecar collision at {remote_name}.bloom: crc mismatch"
                )));
            }
            // RFC-0093 P1.2: CRC match is not a collision theorem (R-crc).
            // Byte-equal after the CRC gate; checking bytes first would
            // hide the P0 same-length XOR tooth.
            if have != bytes {
                return Err(CoreError::CorruptHistory(format!(
                    "remote sidecar collision at {remote_name}.bloom: read-back differs"
                )));
            }
            return Ok(());
        }
        {
            let mut out = remote_env.create(&dest)?;
            out.write_all(&bytes)?;
            out.sync_all()?;
        }
        remote_env.sync_dir(&self.root)?;
        Ok(())
    }

    /// Read the bloom sidecar of remote segment `name` (`None` when the
    /// object is absent — a pre-P2.7 upload or a pre-P2.6 segment).
    /// Other errors propagate; callers treat any sidecar problem as
    /// fail-open (walk).
    pub fn read_sidecar<E: Env>(&self, env: &E, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.segment_path(&format!("{name}.bloom"));
        if !env.exists(&path) {
            return Ok(None);
        }
        let mut f = env.open_read(&path)?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf)?;
        Ok(Some(buf))
    }

    /// Read back one segment object (restore path; the caller replays via
    /// [`walk_segment_records`]).
    pub fn read_segment<E: Env>(&self, env: &E, name: &str) -> Result<Vec<u8>> {
        let mut f = env.open_read(&self.segment_path(name))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf)?;
        Ok(buf)
    }

    /// Upload manifest `bytes` as immutable generation `n`, then point
    /// `LATEST` at it. A crash between the two leaves the previous
    /// `LATEST` — the next upload repairs; readers fall back.
    pub fn put_manifest<E: Env>(&self, env: &E, bytes: &[u8], n: u64) -> Result<PutStatus> {
        env.create_dir_all(&self.root)?;
        let gen = self.segment_path(&Self::manifest_name(n));
        let status = if env.exists(&gen) {
            let mut f = env.open_read(&gen)?;
            let mut have = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut have)?;
            if have == bytes {
                PutStatus::AlreadyPresent
            } else {
                return Err(CoreError::CorruptHistory(format!(
                    "remote manifest generation {n} exists with different bytes"
                )));
            }
        } else {
            let mut f = env.create(&gen)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            PutStatus::Uploaded
        };
        let latest = format!("{}\n{:08x}", Self::manifest_name(n), crc32c(bytes));
        {
            let mut f = env.create(&self.segment_path("LATEST"))?;
            f.write_all(latest.as_bytes())?;
            f.sync_all()?;
        }
        env.sync_dir(&self.root)?;
        Ok(status)
    }

    fn manifest_name(n: u64) -> String {
        format!("MANIFEST-{n:016}")
    }

    /// `LATEST` body: `MANIFEST-<n>\n<crc32c hex of that generation>`.
    fn parse_latest_pointer(buf: &str) -> Option<(&str, u32)> {
        let (name, crc_hex) = buf.trim_end().split_once('\n')?;
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains('\0') {
            return None;
        }
        let crc = u32::from_str_radix(crc_hex.trim(), 16).ok()?;
        Some((name, crc))
    }

    /// Segments of the newest intact remote manifest, oldest-first.
    /// Empty when the remote tier holds no manifest yet.
    pub fn latest_segments<E: Env>(&self, env: &E) -> Result<Vec<RemoteSegment>> {
        let Some(bytes) = self.latest_manifest(env)? else {
            return Ok(Vec::new());
        };
        let manifest = Manifest::decode(&bytes)?;
        Ok(manifest
            .segs
            .into_iter()
            .map(|s| RemoteSegment {
                name: s.name,
                from_seq: s.from_seq,
                through_seq: s.through_seq,
                bytes: s.bytes,
                key_lo: s.key_lo,
                key_hi: s.key_hi,
            })
            .collect())
    }

    /// Roll-up of the newest intact remote manifest for status output
    /// (`None` when the tier holds no manifest).
    pub fn latest_summary<E: Env>(&self, env: &E) -> Result<Option<RemoteSummary>> {
        let Some(bytes) = self.latest_manifest(env)? else {
            return Ok(None);
        };
        let manifest = Manifest::decode(&bytes)?;
        Ok(Some(RemoteSummary {
            segments: manifest.segs.len(),
            bytes: manifest.segs.iter().map(|s| s.bytes).sum(),
            from_seq: manifest.segs.front().map(|s| s.from_seq).unwrap_or(0),
            through_seq: manifest.segs.back().map(|s| s.through_seq).unwrap_or(0),
            archive_floor: manifest.archive_floor,
            next_generation: manifest.next_id,
        }))
    }

    /// RFC-0060 P2.11: CRC-walk every segment (and bloom sidecar) listed
    /// by the newest intact remote manifest. Empty tier is clean.
    ///
    /// # Errors
    /// Remote I/O or a corrupt manifest (not a per-segment CRC miss —
    /// those are counted in [`RemoteVerifyReport::errors`]).
    pub fn verify<E: Env>(&self, env: &E) -> Result<RemoteVerifyReport> {
        let mut report = RemoteVerifyReport::default();
        self.verify_latest_pointer(env, &mut report);
        let segs = self.latest_segments(env)?;
        for seg in segs {
            report.segments = report.segments.saturating_add(1);
            let bytes = match self.read_segment(env, &seg.name) {
                Ok(b) => b,
                Err(e) => {
                    report.errors = report.errors.saturating_add(1);
                    report.failures.push((seg.name.clone(), e.to_string()));
                    continue;
                }
            };
            if let Err(e) = walk_segment_records(&bytes) {
                report.errors = report.errors.saturating_add(1);
                report.failures.push((seg.name.clone(), e.to_string()));
            }
            if let Ok(Some(sc)) = self.read_sidecar(env, &seg.name) {
                report.sidecars = report.sidecars.saturating_add(1);
                if let Err(e) = verify_bloom_sidecar(&sc) {
                    report.errors = report.errors.saturating_add(1);
                    report
                        .failures
                        .push((format!("{}.bloom", seg.name), e.to_string()));
                }
            }
        }
        Ok(report)
    }

    /// RFC-0089: LATEST CRC is `crc_match_ok` (mismatch is never a clean
    /// verify). Torn/unreadable pointer still counts as a named failure;
    /// the reader walks back separately.
    fn verify_latest_pointer<E: Env>(&self, env: &E, report: &mut RemoteVerifyReport) {
        let latest = self.segment_path("LATEST");
        if !env.exists(&latest) {
            return;
        }
        let mut buf = String::new();
        let Ok(mut f) = env.open_read(&latest) else {
            report.errors = report.errors.saturating_add(1);
            report.failures.push(("LATEST".into(), "unreadable".into()));
            return;
        };
        if f.read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
            report.errors = report.errors.saturating_add(1);
            report.failures.push(("LATEST".into(), "empty".into()));
            return;
        }
        let Some((name, expect_crc)) = Self::parse_latest_pointer(&buf) else {
            report.errors = report.errors.saturating_add(1);
            report
                .failures
                .push(("LATEST".into(), "bad pointer".into()));
            return;
        };
        let p = self.segment_path(name);
        if !env.exists(&p) {
            report.errors = report.errors.saturating_add(1);
            report
                .failures
                .push(("LATEST".into(), format!("missing {name}")));
            return;
        }
        let Ok(mut mf) = env.open_read(&p) else {
            return;
        };
        let mut mb = Vec::new();
        if std::io::Read::read_to_end(&mut mf, &mut mb).is_err() {
            return;
        }
        if !crate::wal::crc::crc_match_ok(crc32c(&mb), expect_crc) {
            report.errors = report.errors.saturating_add(1);
            report
                .failures
                .push(("LATEST".into(), "crc mismatch".into()));
        }
    }

    /// Newest intact manifest generation: `LATEST` if it parses and its
    /// target decodes; otherwise the highest-numbered intact generation;
    /// `None` when the remote tier is empty.
    /// RFC-0089 P1.2: a CRC-hex lie that names an older generation must
    /// not serve that generation — walk back to the newest intact file.
    pub fn latest_manifest<E: Env>(&self, env: &E) -> Result<Option<Vec<u8>>> {
        let latest = self.segment_path("LATEST");
        if env.exists(&latest) {
            if let Ok(mut f) = env.open_read(&latest) {
                let mut buf = String::new();
                if f.read_to_string(&mut buf).is_ok_and(|_| !buf.is_empty()) {
                    if let Some((name, expect_crc)) = Self::parse_latest_pointer(&buf) {
                        let p = self.segment_path(name);
                        if env.exists(&p) {
                            if let Ok(mut mf) = env.open_read(&p) {
                                let mut mb = Vec::new();
                                if std::io::Read::read_to_end(&mut mf, &mut mb).is_ok()
                                    && Manifest::decode(&mb).is_ok()
                                    && crate::wal::crc::crc_match_ok(crc32c(&mb), expect_crc)
                                {
                                    return Ok(Some(mb));
                                }
                            }
                        }
                    }
                }
            }
            // Torn/garbage LATEST — fall through to generation walk-back.
        }
        let mut gens: Vec<String> = env
            .read_dir_names(&self.root)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.starts_with("MANIFEST-"))
            .collect();
        gens.sort();
        while let Some(name) = gens.pop() {
            let p = self.segment_path(&name);
            if let Ok(mut mf) = env.open_read(&p) {
                let mut mb = Vec::new();
                if std::io::Read::read_to_end(&mut mf, &mut mb).is_ok()
                    && Manifest::decode(&mb).is_ok()
                {
                    return Ok(Some(mb));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::rc::Rc;

    /// In-memory `Env` (flat namespace, dir names derived from parents).
    /// Writes commit to the map on sync and on drop.
    #[derive(Clone, Default)]
    struct MapEnv {
        files: Rc<RefCell<BTreeMap<PathBuf, Vec<u8>>>>,
    }

    struct MapFile {
        files: Rc<RefCell<BTreeMap<PathBuf, Vec<u8>>>>,
        path: PathBuf,
        buf: Vec<u8>,
        pos: usize,
    }

    impl MapFile {
        fn commit(&self) {
            self.files
                .borrow_mut()
                .insert(self.path.clone(), self.buf.clone());
        }
    }

    impl Drop for MapFile {
        fn drop(&mut self) {
            self.commit();
        }
    }

    impl std::io::Read for MapFile {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min(self.buf.len().saturating_sub(self.pos));
            out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl std::io::Seek for MapFile {
        fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
            let p: i64 = match to {
                std::io::SeekFrom::Start(s) => s as i64,
                std::io::SeekFrom::Current(d) => self.pos as i64 + d,
                std::io::SeekFrom::End(d) => self.buf.len() as i64 + d,
            };
            self.pos = p.max(0) as usize;
            Ok(self.pos as u64)
        }
    }

    impl std::io::Write for MapFile {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if self.pos > self.buf.len() {
                self.buf.resize(self.pos, 0);
            }
            let end = self.pos + data.len();
            if end > self.buf.len() {
                self.buf.resize(end, 0);
            }
            self.buf[self.pos..end].copy_from_slice(data);
            self.pos = end;
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl EnvFile for MapFile {
        fn sync_data(&mut self) -> std::io::Result<()> {
            self.commit();
            Ok(())
        }
        fn sync_all(&mut self) -> std::io::Result<()> {
            self.commit();
            Ok(())
        }
        fn set_len(&mut self, len: u64) -> std::io::Result<()> {
            self.buf.resize(len as usize, 0);
            self.pos = self.pos.min(self.buf.len());
            self.commit();
            Ok(())
        }
        fn len(&mut self) -> std::io::Result<u64> {
            Ok(self.buf.len() as u64)
        }
    }

    impl Env for MapEnv {
        type File = MapFile;
        fn create_dir_all(&self, _path: &Path) -> std::io::Result<()> {
            Ok(())
        }
        fn create(&self, path: &Path) -> std::io::Result<Self::File> {
            self.files.borrow_mut().remove(path);
            Ok(MapFile {
                files: Rc::clone(&self.files),
                path: path.to_path_buf(),
                buf: Vec::new(),
                pos: 0,
            })
        }
        fn open_append(&self, path: &Path) -> std::io::Result<Self::File> {
            let buf = self.files.borrow().get(path).cloned().unwrap_or_default();
            let pos = buf.len();
            Ok(MapFile {
                files: Rc::clone(&self.files),
                path: path.to_path_buf(),
                buf,
                pos,
            })
        }
        fn open_read(&self, path: &Path) -> std::io::Result<Self::File> {
            let buf = self
                .files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))?;
            Ok(MapFile {
                files: Rc::clone(&self.files),
                path: path.to_path_buf(),
                buf,
                pos: 0,
            })
        }
        fn sync_dir(&self, _path: &Path) -> std::io::Result<()> {
            Ok(())
        }
        fn read_dir_names(&self, path: &Path) -> std::io::Result<Vec<String>> {
            let names = self
                .files
                .borrow()
                .keys()
                .filter(|p| p.parent() == Some(path))
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
            Ok(names)
        }
        fn remove_file(&self, path: &Path) -> std::io::Result<()> {
            self.files
                .borrow_mut()
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            let mut files = self.files.borrow_mut();
            match files.remove(from) {
                Some(b) => {
                    files.insert(to.to_path_buf(), b);
                    Ok(())
                }
                None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
            }
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
        }
        fn metadata_len(&self, path: &Path) -> std::io::Result<u64> {
            self.files
                .borrow()
                .get(path)
                .map(|b| b.len() as u64)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
    }

    /// `FailingEnv` role: one-shot create faults against a `MapEnv`.
    #[derive(Clone)]
    struct FaultyEnv {
        inner: MapEnv,
        fail_create: Rc<Cell<bool>>,
    }

    impl FaultyEnv {
        fn new(inner: MapEnv) -> Self {
            Self {
                inner,
                fail_create: Rc::new(Cell::new(false)),
            }
        }
    }

    impl Env for FaultyEnv {
        type File = MapFile;
        fn create_dir_all(&self, p: &Path) -> std::io::Result<()> {
            self.inner.create_dir_all(p)
        }
        fn create(&self, p: &Path) -> std::io::Result<Self::File> {
            if self.fail_create.get() {
                return Err(std::io::Error::other("injected create failure"));
            }
            self.inner.create(p)
        }
        fn open_append(&self, p: &Path) -> std::io::Result<Self::File> {
            self.inner.open_append(p)
        }
        fn open_read(&self, p: &Path) -> std::io::Result<Self::File> {
            self.inner.open_read(p)
        }
        fn sync_dir(&self, p: &Path) -> std::io::Result<()> {
            self.inner.sync_dir(p)
        }
        fn read_dir_names(&self, p: &Path) -> std::io::Result<Vec<String>> {
            self.inner.read_dir_names(p)
        }
        fn remove_file(&self, p: &Path) -> std::io::Result<()> {
            self.inner.remove_file(p)
        }
        fn rename(&self, f: &Path, t: &Path) -> std::io::Result<()> {
            self.inner.rename(f, t)
        }
        fn exists(&self, p: &Path) -> bool {
            self.inner.exists(p)
        }
        fn metadata_len(&self, p: &Path) -> std::io::Result<u64> {
            self.inner.metadata_len(p)
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "pedradb-hist-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// Local tier with three archived versions of `k` (real files on disk).
    fn seeded_tier(tag: &str) -> (PathBuf, HistoryTier) {
        let root = temp_root(tag);
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        let records = vec![
            (b"k".to_vec(), b"v1".to_vec(), 1u64, 0u8),
            (b"k".to_vec(), b"v2".to_vec(), 2, 0),
            (b"k".to_vec(), b"v3".to_vec(), 3, 0),
        ];
        tier.archive_stream(&crate::env::StdEnv, records.into_iter())
            .unwrap();
        (root, tier)
    }

    fn only_segment_path(root: &Path) -> PathBuf {
        let dir = root.join("history");
        let segs: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("seg-") && n.ends_with(".hist"))
            .collect();
        assert_eq!(segs.len(), 1, "seeded tier archives one segment");
        dir.join(&segs[0])
    }

    fn only_bloom_path(root: &Path) -> PathBuf {
        let dir = root.join("history");
        let blooms: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("seg-") && n.ends_with(".bloom"))
            .collect();
        assert_eq!(blooms.len(), 1, "seeded tier writes one bloom sidecar");
        dir.join(&blooms[0])
    }

    fn remote_objects(map: &MapEnv) -> Vec<String> {
        map.read_dir_names(Path::new("/remote")).unwrap()
    }

    const REMOTE: &str = "/remote";

    /// RFC-0086 P0 / RFC-0082 P1.2: production `archive_stream` writes
    /// `history/MANIFEST`; XOR only the trailer CRC (payload intact).
    /// Decode/open is crc mismatch. AS-IS would load the inventory.
    #[test]
    fn crc_mismatch_on_live_history_manifest_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any history manifest crc would match"
        );
        let (root, _tier) = seeded_tier("crc-0086");
        let path = root.join("history").join("MANIFEST");
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() >= 8,
            "history MANIFEST must have payload + trailer"
        );
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let err = verify_history_manifest(&bytes).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a payload parse; got {msg}"
        );
        let open_err = HistoryTier::open(&crate::env::StdEnv, &root).unwrap_err();
        assert!(
            open_err
                .to_string()
                .to_ascii_lowercase()
                .contains("crc mismatch"),
            "HistoryTier::open must refuse the trailer lie; got {open_err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RFC-0086 P2.2: history MANIFEST `crc_match_ok` is not a CRC32C
    /// collision theorem (R-crc stays never_floor).
    #[test]
    fn history_manifest_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0087 P0: production `archive_stream` writes `seg-*.hist`; XOR
    /// only the last record's CRC trailer (key/len intact). Walk is crc
    /// mismatch. AS-IS would return the archived versions.
    #[test]
    fn crc_mismatch_on_live_history_segment_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any segment crc would match"
        );
        let (root, _tier) = seeded_tier("crc-0087");
        let path = only_segment_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() >= 4, "segment must have a record CRC trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = walk_segment_records(&bytes).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a key/len parse; got {msg}"
        );
    }

    /// RFC-0087 P2.1: segment `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn history_segment_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0087 P2.2: upload (`put_segment`) and restore/scrub callers stay
    /// on `walk_segment_records`. Same last-record CRC trailer lie as P0:
    /// put is crc mismatch (nothing uploaded); `verify_at_rest` names the
    /// `.hist` file. Payload flip (`remote_segment_put_refuses_corrupt_local`)
    /// is not this tooth.
    #[test]
    fn history_segment_upload_restore_stay_on_walk() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any segment crc would match"
        );
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for rel in [
            "src/history.rs",
            "src/db.rs",
            "src/verify.rs",
            "../pedradb-ops/src/lib.rs",
        ] {
            let text = std::fs::read_to_string(crate_root.join(rel))
                .unwrap_or_else(|e| panic!("read {rel}: {e}"));
            assert!(
                text.contains("walk_segment_records"),
                "{rel} must stay on walk_segment_records"
            );
        }

        let (local, _tier) = seeded_tier("crc-0087-up");
        let seg = only_segment_path(&local);
        let mut bytes = std::fs::read(&seg).unwrap();
        assert!(bytes.len() >= 4, "segment must have a record CRC trailer");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&seg, &bytes).unwrap();
        let remote_root = temp_root("crc-0087-up-r");
        let remote = RemoteTier::new(&remote_root);
        let err = remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap_err();
        let msg = err.to_string();
        let uploaded = std::fs::read_dir(&remote_root)
            .map(|rd| {
                rd.filter_map(|e| e.ok()).any(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    n.starts_with("seg-") && n.ends_with(".hist")
                })
            })
            .unwrap_or(false);
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "put_segment must fail on crc_match_ok, not a key/len parse; got {msg}"
        );
        assert!(!uploaded, "nothing may be uploaded from a CRC-lied segment");

        let (root, _tier) = seeded_tier("crc-0087-sc");
        let path = only_segment_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let r = crate::verify::verify_at_rest(&crate::env::StdEnv, &root);
        let rel = format!("history/{}", path.file_name().unwrap().to_string_lossy());
        let _ = std::fs::remove_dir_all(&root);
        assert!(!r.is_clean(), "trailer lie must fail the scrub");
        assert!(
            r.failures.iter().any(|f| {
                f.file == rel && f.message.to_ascii_lowercase().contains("crc mismatch")
            }),
            "must name {rel} crc mismatch, got {:?}",
            r.failures
        );
    }

    /// RFC-0088 P0: production `archive_stream` writes `seg-*.bloom`; XOR
    /// only the trailer CRC (magic / body_len / bits intact). Verify is crc
    /// mismatch. AS-IS would report the sidecar clean.
    #[test]
    fn crc_mismatch_on_live_history_bloom_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any bloom sidecar crc would match"
        );
        let (root, _tier) = seeded_tier("crc-0088");
        let path = only_bloom_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() >= 8 + 12,
            "bloom sidecar must have PHB1 header + body + footer"
        );
        assert_eq!(&bytes[0..4], b"PHB1", "live sidecar magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = verify_bloom_sidecar(&bytes).unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a magic/len parse; got {msg}"
        );
    }

    /// RFC-0088 P1.1: production prune parse (`sidecar_may_affect`) uses
    /// `crc_match_ok`. XOR only the trailer CRC (magic/body_len/bits
    /// intact). Intact bloom prunes `zzz`; after the lie, still walks.
    /// AS-IS would prune. `segment_may_affect` file I/O is RFC-0091 P2.1.
    #[test]
    fn crc_mismatch_on_live_history_bloom_sidecar_may_affect_still_walks() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any bloom sidecar crc would match"
        );
        let (root, _tier) = seeded_tier("crc-0088-p11");
        let path = only_bloom_path(&root);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"PHB1", "live sidecar magic");
        assert!(
            !HistoryTier::sidecar_may_affect(&bytes, b"zzz"),
            "intact bloom must prune zzz so the CRC-lie tooth is observable"
        );
        let mut lied = bytes.clone();
        let last = lied.len() - 1;
        lied[last] ^= 0xff;
        let walks = HistoryTier::sidecar_may_affect(&lied, b"zzz");
        let _ = std::fs::remove_dir_all(&root);
        assert!(walks, "CRC trailer lie must fail-open (walk), never prune");
    }

    /// RFC-0088 P2.1: bloom sidecar `crc_match_ok` is not a CRC32C
    /// collision theorem.
    #[test]
    fn history_bloom_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0088 P2.2: prune stays fail-open. Same trailer lie: scrub
    /// (`verify_bloom_sidecar`) is crc mismatch; prune still walks.
    /// `db.rs` stays on `sidecar_may_affect`. Do not make prune fail-closed.
    #[test]
    fn history_bloom_crc_mismatch_prune_stays_fail_open() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any bloom sidecar crc would match"
        );
        let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let db = std::fs::read_to_string(crate_root.join("src/db.rs")).expect("db.rs");
        assert!(
            db.contains("sidecar_may_affect"),
            "db.rs restore prune must stay on sidecar_may_affect"
        );
        let hist = std::fs::read_to_string(crate_root.join("src/history.rs")).expect("history.rs");
        assert!(
            hist.contains("return true; // corrupt sidecar"),
            "mismatch must keep the fail-open walk, not prune"
        );

        let (root, _tier) = seeded_tier("crc-0088-p22");
        let path = only_bloom_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"PHB1", "live sidecar magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let scrub = verify_bloom_sidecar(&bytes).unwrap_err().to_string();
        let walks = HistoryTier::sidecar_may_affect(&bytes, b"zzz");
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            scrub.to_ascii_lowercase().contains("crc mismatch"),
            "scrub stays fail-closed; got {scrub}"
        );
        assert!(walks, "prune stays fail-open (walk), never skip on CRC lie");
    }

    /// RFC-0091 P2.1: production prune (`segment_may_affect`) XOR only
    /// the sidecar trailer CRC (magic/body_len/bits intact). Must still
    /// walk (fail-open). AS-IS would prune. `verify_bloom_sidecar` is
    /// RFC-0088, not this tooth. Body-byte flip in
    /// `bloom_sidecar_missing_or_corrupt_never_prunes` is not this tooth.
    #[test]
    fn crc_mismatch_on_live_history_bloom_prune_still_walks() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any bloom sidecar crc would match"
        );
        let (root, tier) = seeded_tier("crc-0091-bloom");
        let id = tier.segment_metas()[0].id;
        let prune_key: &[u8] = b"zzz";
        assert!(
            !tier.segment_may_affect(&crate::env::StdEnv, id, prune_key),
            "intact bloom must prune {prune_key:?} so the CRC-lie tooth is observable"
        );
        let path = only_bloom_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"PHB1", "live sidecar magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        let walks = tier.segment_may_affect(&crate::env::StdEnv, id, prune_key);
        let _ = std::fs::remove_dir_all(&root);
        assert!(walks, "CRC trailer lie must fail-open (walk), never prune");
    }

    /// RFC-0089 P0: production `put_manifest` writes `LATEST`; XOR only
    /// the stored CRC u32 and rewrite as 8 hex digits (name intact,
    /// MANIFEST bytes intact). Verify names crc mismatch. AS-IS would
    /// report the pointer clean. XOR of an ASCII hex digit is not this
    /// tooth (`bad pointer`). `remote_verify_flags_corrupt_latest`
    /// (`ffffffff` rewrite) is RFC-0060 P2.12, not this gate.
    #[test]
    fn crc_mismatch_on_live_history_latest_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any LATEST crc would match"
        );
        let local = temp_root("crc-0089-l");
        let remote_root = temp_root("crc-0089-r");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &local).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v1".to_vec(), 1u64, 0u8)].into_iter(),
        )
        .unwrap();
        let seg = only_segment_path(&local);
        let remote = RemoteTier::new(&remote_root);
        remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap();
        let man = std::fs::read(local.join("history").join("MANIFEST")).unwrap();
        remote
            .put_manifest(&crate::env::StdEnv, &man, tier.remote_generation())
            .unwrap();
        let latest = remote_root.join("LATEST");
        let body = std::fs::read_to_string(&latest).unwrap();
        let (name, crc_hex) = body.trim_end().split_once('\n').expect("LATEST pointer");
        let crc = u32::from_str_radix(crc_hex.trim(), 16).expect("LATEST crc hex");
        std::fs::write(&latest, format!("{name}\n{:08x}", crc ^ 0xffff_ffff)).unwrap();
        let r = remote.verify(&crate::env::StdEnv).unwrap();
        assert!(!r.is_clean(), "LATEST crc-hex lie must fail verify");
        assert!(
            r.failures
                .iter()
                .any(|(f, m)| { f == "LATEST" && m.to_ascii_lowercase().contains("crc mismatch") }),
            "must fail on crc_match_ok, not a pointer parse; got {:?}",
            r.failures
        );
        let got = remote.latest_manifest(&crate::env::StdEnv).unwrap();
        assert_eq!(
            got.as_deref(),
            Some(man.as_slice()),
            "walk-back still serves the intact generation"
        );
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
    }

    /// RFC-0089 P1.2: two remote generations. `LATEST` names the older
    /// with XOR'd CRC hex (name intact, both MANIFEST files intact).
    /// `latest_manifest` walk-back serves the newest intact (m2), never
    /// the named older (m1). AS-IS `crc_match_ok` would serve m1.
    /// `ffffffff` rewrite (`remote_manifest_generations_latest_and_walkback`)
    /// is RFC-0060 P2.12, not this tooth.
    #[test]
    fn crc_mismatch_on_live_history_latest_walkback_refuses_named_older() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any LATEST crc would match"
        );
        let (root, mut tier) = seeded_tier("crc-0089-p12");
        let remote_root = temp_root("crc-0089-p12-r");
        let remote = RemoteTier::new(&remote_root);
        let m1 = tier.manifest_bytes();
        let n1 = tier.remote_generation();
        remote.put_manifest(&crate::env::StdEnv, &m1, n1).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v4".to_vec(), 4, 0)].into_iter(),
        )
        .unwrap();
        let m2 = tier.manifest_bytes();
        let n2 = tier.remote_generation();
        assert_ne!(n1, n2, "second archive must mint a new generation");
        assert_ne!(m1, m2, "manifest bytes must differ across generations");
        remote.put_manifest(&crate::env::StdEnv, &m2, n2).unwrap();
        let latest = remote_root.join("LATEST");
        let stored = crc32c(&m1);
        std::fs::write(
            &latest,
            format!("MANIFEST-{n1:016}\n{:08x}", stored ^ 0xffff_ffff),
        )
        .unwrap();
        let got = remote.latest_manifest(&crate::env::StdEnv).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert_eq!(
            got.as_deref(),
            Some(m2.as_slice()),
            "walk-back must serve the newest intact generation, not the named older"
        );
        assert_ne!(
            got.as_deref(),
            Some(m1.as_slice()),
            "CRC-hex lie must not serve the named older generation"
        );
    }

    /// RFC-0089 P2.1: LATEST `crc_match_ok` is not a CRC32C collision theorem.
    #[test]
    fn history_latest_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0089 P2.2: content-addressed put read-back identity stays
    /// RFC-0092 (`crc_match_ok` on two computed CRCs). This RFC is the
    /// LATEST hex trailer, not put resume. Byte-equal is RFC-0092 P2.1.
    #[test]
    fn history_latest_put_readback_stays_rfc0092() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        let hist = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/history.rs"),
        )
        .expect("history.rs");
        assert!(
            hist.contains("crc_mismatch_on_live_history_put_is_not_ok"),
            "put resume identity stays RFC-0092"
        );
        assert!(
            hist.contains("crc_match_ok(crc32c(&have), crc32c(&bytes))"),
            "put_segment identity is two computed CRCs, not LATEST hex"
        );
        assert!(
            hist.contains("fn verify_latest_pointer"),
            "LATEST hex trailer stays this RFC"
        );
    }

    #[test]
    fn segment_key_coverage_prunes_reads_soundly() {
        // P2.5: segments carry a key-coverage bound; a read for a key
        // outside it never touches the file. Observable: corrupt the
        // out-of-range segment — the read must still succeed; corrupt the
        // in-range one — the read must fail CorruptHistory (fail-closed
        // unchanged for what it actually reads).
        let root = temp_root("prune");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![
                (b"aa".to_vec(), b"v1".to_vec(), 1u64, 0u8),
                (b"ab".to_vec(), b"v2".to_vec(), 2, 0),
            ]
            .into_iter(),
        )
        .unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"zz".to_vec(), b"w1".to_vec(), 3u64, 0u8)].into_iter(),
        )
        .unwrap();
        let metas = tier.segment_metas();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].key_lo.as_deref(), Some(&b"aa"[..]));
        assert_eq!(metas[0].key_hi.as_deref(), Some(&b"ab"[..]));
        assert_eq!(metas[1].key_lo.as_deref(), Some(&b"zz"[..]));
        assert_eq!(metas[1].key_hi.as_deref(), Some(&b"zz"[..]));
        // Corrupt segment 1 (aa..ab): a read inside its range fails typed.
        let seg1 = root
            .join("history")
            .join(format!("seg-{:08}.hist", metas[0].id));
        let mut bytes = std::fs::read(&seg1).unwrap();
        bytes[10] ^= 0xff;
        std::fs::write(&seg1, &bytes).unwrap();
        let records = crate::history::walk_segment_records(
            &tier
                .read_local_segment(&crate::env::StdEnv, metas[0].id)
                .unwrap()
                .expect("present"),
        );
        assert!(
            records.is_err(),
            "in-range read still verifies CRC (fail-closed)"
        );
        // Coverage of segment 2 excludes aa/ab — the db-level reader would
        // skip it; here we assert the bound math directly: aa < zz and
        // ab < zz, so both prune, while zz does not.
        let covers = |m: &SegmentMeta, k: &[u8]| {
            m.key_lo.as_ref().is_some_and(|lo| {
                m.key_hi
                    .as_ref()
                    .is_some_and(|hi| k >= lo.as_slice() && k <= hi.as_slice())
            })
        };
        assert!(!covers(&metas[1], b"aa") && !covers(&metas[1], b"ab"));
        assert!(covers(&metas[1], b"zz"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn segment_key_coverage_counts_range_delete_ends() {
        // Soundness: a range delete [a, z) affects every key in between
        // while its record key is only "a". The ceiling must include the
        // exclusive end, or reads for interior keys would prune the
        // segment and lose the delete.
        let root = temp_root("rdc");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"a".to_vec(), b"z".to_vec(), 7u64, 2u8)].into_iter(),
        )
        .unwrap();
        let metas = tier.segment_metas();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].key_lo.as_deref(), Some(&b"a"[..]));
        assert_eq!(
            metas[0].key_hi.as_deref(),
            Some(&b"z"[..]),
            "ceiling must cover the range-delete end, not just record keys"
        );
        // Interior key is inside coverage; decide_at applies the delete.
        let bytes = tier
            .read_local_segment(&crate::env::StdEnv, metas[0].id)
            .unwrap()
            .expect("present");
        let records = crate::history::walk_segment_records(&bytes).unwrap();
        let decided = crate::history::decide_at(&records, b"m", 10).expect("covered key decides");
        assert_eq!(decided.kind, 2, "range delete covers the interior key");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_v2_decodes_without_key_coverage() {
        // Backward compat: manifests sealed before P2.5 (version 2) decode
        // with None coverage — those segments always walk, never mis-prune.
        let bad = |v: u32| {
            let mut b = Vec::new();
            b.extend_from_slice(b"PHST");
            b.extend_from_slice(&v.to_le_bytes());
            b.extend_from_slice(&1u64.to_le_bytes()); // next_id
            b.extend_from_slice(&0u64.to_le_bytes()); // archive_floor
            b.extend_from_slice(&1u32.to_le_bytes()); // one segment
            b.extend_from_slice(&7u64.to_le_bytes()); // id
            b.extend_from_slice(&4u32.to_le_bytes()); // name len
            b.extend_from_slice(b"segx");
            b.extend_from_slice(&1u64.to_le_bytes()); // from_seq
            b.extend_from_slice(&2u64.to_le_bytes()); // through_seq
            b.extend_from_slice(&64u64.to_le_bytes()); // bytes
            let crc = crc32c(&b);
            b.extend_from_slice(&crc.to_le_bytes());
            b
        };
        let m2 = super::Manifest::decode(&bad(2)).expect("v2 decodes");
        assert_eq!(m2.segs.len(), 1);
        assert!(m2.segs[0].key_lo.is_none() && m2.segs[0].key_hi.is_none());
        assert!(
            super::Manifest::decode(&bad(4)).is_err(),
            "unknown version still rejected"
        );
        // v3 round-trip keeps coverage.
        let mut m3 = super::Manifest::decode(&bad(2)).unwrap();
        m3.segs[0].key_lo = Some(b"lo".to_vec());
        m3.segs[0].key_hi = Some(b"hi".to_vec());
        let rt = super::Manifest::decode(&m3.encode()).unwrap();
        assert_eq!(rt.segs[0].key_lo.as_deref(), Some(&b"lo"[..]));
        assert_eq!(rt.segs[0].key_hi.as_deref(), Some(&b"hi"[..]));
    }

    #[test]
    fn bloom_sidecar_prunes_overlapping_key_ranges() {
        // P2.6: the case the P2.5 manifest bound can't prune — segments
        // whose key ranges overlap (overwrite workloads). The bloom must
        // answer "cannot affect" for a key inside the overlap but in no
        // segment, and "may affect" for keys actually present.
        let root = temp_root("blm");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![
                (b"aa".to_vec(), b"v1".to_vec(), 1u64, 0u8),
                (b"ab".to_vec(), b"v2".to_vec(), 2, 0),
            ]
            .into_iter(),
        )
        .unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![
                (b"aa".to_vec(), b"v3".to_vec(), 3u64, 0u8),
                (b"ab".to_vec(), b"v4".to_vec(), 4, 0),
            ]
            .into_iter(),
        )
        .unwrap();
        let metas = tier.segment_metas();
        assert_eq!(metas.len(), 2, "two overlapping segments");
        // Overlapping manifest coverage (P2.5 can't prune either).
        assert!(metas
            .iter()
            .all(|m| m.key_lo.as_deref() == Some(&b"aa"[..])));
        for m in &metas {
            assert!(
                root.join("history")
                    .join(format!("seg-{:08}.bloom", m.id))
                    .exists(),
                "seal writes the bloom sidecar"
            );
            assert!(!tier.segment_may_affect(&crate::env::StdEnv, m.id, b"az"));
            assert!(tier.segment_may_affect(&crate::env::StdEnv, m.id, b"aa"));
            assert!(tier.segment_may_affect(&crate::env::StdEnv, m.id, b"ab"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bloom_sidecar_range_delete_intervals_sound() {
        // A range delete [a, z) decides every interior key while its record
        // key is only "a": the sidecar keeps explicit intervals, so interior
        // keys are never pruned and outside keys are.
        let root = temp_root("blrd");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![
                (b"b".to_vec(), b"v".to_vec(), 1u64, 0u8),
                (b"a".to_vec(), b"z".to_vec(), 2, 2), // RD [a, z)
                (b"zz".to_vec(), b"w".to_vec(), 3, 0),
            ]
            .into_iter(),
        )
        .unwrap();
        let m = &tier.segment_metas()[0];
        assert!(tier.segment_may_affect(&crate::env::StdEnv, m.id, b"m"));
        assert!(tier.segment_may_affect(&crate::env::StdEnv, m.id, b"y"));
        assert!(!tier.segment_may_affect(&crate::env::StdEnv, m.id, b"zzz"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bloom_sidecar_missing_or_corrupt_never_prunes() {
        // Fail-open by construction: a damaged or absent sidecar must read
        // as "may affect" (walk). Also: an orphan sidecar (no manifest
        // entry) is removed at open.
        let root = temp_root("blx");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v".to_vec(), 1u64, 0u8)].into_iter(),
        )
        .unwrap();
        let id = tier.segment_metas()[0].id;
        let sidecar = root.join("history").join(format!("seg-{id:08}.bloom"));
        // Missing → walk.
        std::fs::remove_file(&sidecar).unwrap();
        assert!(tier.segment_may_affect(&crate::env::StdEnv, id, b"unrelated"));
        // Corrupt body (flip a byte inside the bloom bits) → walk.
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v".to_vec(), 2u64, 0u8)].into_iter(),
        )
        .unwrap();
        let id2 = tier.segment_metas()[1].id;
        let sc2 = root.join("history").join(format!("seg-{id2:08}.bloom"));
        let mut bytes = std::fs::read(&sc2).unwrap();
        // Flip a byte inside the body region (past the 8-byte header,
        // before the 12-byte footer).
        bytes[13] ^= 0xff;
        std::fs::write(&sc2, &bytes).unwrap();
        assert!(tier.segment_may_affect(&crate::env::StdEnv, id2, b"unrelated"));
        // Orphan sidecar (id not in the manifest) → removed at the next
        // open; live sidecars survive it.
        drop(tier);
        let orphan = root.join("history").join("seg-99999999.bloom");
        std::fs::write(&orphan, b"junk").unwrap();
        let reopened = HistoryTier::open(&crate::env::StdEnv, &root).unwrap();
        assert!(!orphan.exists(), "orphan sidecar cleaned at open");
        assert!(sc2.exists(), "live sidecar kept at open");
        let _ = reopened;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_segment_put_content_addressed_and_idempotent() {
        let (root, _tier) = seeded_tier("ca");
        let seg = only_segment_path(&root);
        let remote = RemoteTier::new(REMOTE);
        let map = MapEnv::default();
        let first = remote.put_segment(&map, &crate::env::StdEnv, &seg).unwrap();
        assert_eq!(first, PutStatus::Uploaded);
        let second = remote.put_segment(&map, &crate::env::StdEnv, &seg).unwrap();
        assert_eq!(second, PutStatus::AlreadyPresent);
        let objects: Vec<String> = remote_objects(&map)
            .into_iter()
            .filter(|n| n.starts_with("seg-") && n.ends_with(".hist"))
            .collect();
        assert_eq!(
            objects.len(),
            1,
            "idempotent put must not duplicate objects"
        );
        let bytes = std::fs::read(&seg).unwrap();
        assert_eq!(objects[0], RemoteTier::segment_name(&bytes));
        // The P2.7 bloom sidecar ships alongside, exactly once, named after the segment.
        let sidecars: Vec<String> = remote_objects(&map)
            .into_iter()
            .filter(|n| n.ends_with(".bloom"))
            .collect();
        assert_eq!(sidecars.len(), 1, "sidecar ships with the segment");
        assert_eq!(
            sidecars[0],
            format!("{}.bloom", RemoteTier::segment_name(&bytes))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_segment_put_refuses_corrupt_local() {
        let (root, _tier) = seeded_tier("corrupt");
        let seg = only_segment_path(&root);
        let mut bytes = std::fs::read(&seg).unwrap();
        // Flip a byte inside the first record body (past the header).
        bytes[10] ^= 0xff;
        std::fs::write(&seg, &bytes).unwrap();
        let remote = RemoteTier::new(REMOTE);
        let map = MapEnv::default();
        let err = remote.put_segment(&map, &crate::env::StdEnv, &seg);
        assert!(
            matches!(err, Err(CoreError::CorruptHistory(_))),
            "corrupt local segment must fail-closed before upload"
        );
        assert!(
            remote_objects(&map).iter().all(|n| !n.starts_with("seg-")),
            "nothing may be uploaded from a corrupt segment"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_segment_name_collision_fails_closed() {
        let (root, _tier) = seeded_tier("coll");
        let seg = only_segment_path(&root);
        let bytes = std::fs::read(&seg).unwrap();
        let map = MapEnv::default();
        // Plant different bytes under the content-addressed name.
        let name = RemoteTier::segment_name(&bytes);
        {
            let mut f = map.create(Path::new(REMOTE).join(&name).as_path()).unwrap();
            f.write_all(b"different bytes entirely").unwrap();
            f.sync_all().unwrap();
        }
        let remote = RemoteTier::new(REMOTE);
        let err = remote.put_segment(&map, &crate::env::StdEnv, &seg);
        assert!(
            matches!(err, Err(CoreError::CorruptHistory(_))),
            "read-back mismatch must be a typed collision error"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RFC-0092 P0: production `put_segment` resumes on name hit; XOR one
    /// payload byte of the remote object (length intact). Re-put is crc
    /// mismatch. AS-IS would return AlreadyPresent.
    /// `remote_segment_name_collision_fails_closed` (different length) is
    /// not this tooth.
    #[test]
    fn crc_mismatch_on_live_history_put_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any same-length remote crc would match"
        );
        let (local, _tier) = seeded_tier("crc-0092");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0092-r");
        let remote = RemoteTier::new(&remote_root);
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::Uploaded
        );
        let bytes = std::fs::read(&seg).unwrap();
        let dest = remote_root.join(RemoteTier::segment_name(&bytes));
        let mut have = std::fs::read(&dest).unwrap();
        assert_eq!(have.len(), bytes.len(), "plant keeps length");
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&dest, &have).unwrap();
        match remote.put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg) {
            Ok(st) => {
                let _ = std::fs::remove_dir_all(&local);
                let _ = std::fs::remove_dir_all(&remote_root);
                panic!("AS-IS hole: {st:?} after same-length CRC lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&local);
                let _ = std::fs::remove_dir_all(&remote_root);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a length parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0092 P1.2: same same-length payload lie as P0; the collision
    /// error names the content-addressed object. Length-mismatch plant
    /// (`remote_segment_name_collision_fails_closed`) is not this tooth.
    #[test]
    fn crc_mismatch_on_live_history_put_names_the_object() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any same-length remote crc would match"
        );
        let (local, _tier) = seeded_tier("crc-0092-p12");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0092-p12-r");
        let remote = RemoteTier::new(&remote_root);
        remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap();
        let bytes = std::fs::read(&seg).unwrap();
        let name = RemoteTier::segment_name(&bytes);
        let dest = remote_root.join(&name);
        let mut have = std::fs::read(&dest).unwrap();
        assert_eq!(have.len(), bytes.len(), "plant keeps length");
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&dest, &have).unwrap();
        let err = remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on crc_match_ok, not a length parse; got {msg}"
        );
        assert!(msg.contains(&name), "collision must name {name}, got {msg}");
    }

    /// RFC-0092 P2.1: resume also requires byte-equal after `crc_match_ok`.
    /// Same-length XOR still fails CRC first (P0 tooth). Identical re-put
    /// is AlreadyPresent. Byte-equal first would hide the AS-IS CRC tooth.
    #[test]
    fn history_put_resume_requires_byte_equal_after_crc() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        let hist = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/history.rs"),
        )
        .expect("history.rs");
        let crc_at = hist
            .find("crc_match_ok(crc32c(&have), crc32c(&bytes))")
            .expect("put_segment CRC identity");
        let bytes_at = hist[crc_at..]
            .find("if have != bytes")
            .expect("byte-equal must follow crc_match_ok in put_segment");
        assert!(
            bytes_at > 0,
            "byte-equal after CRC, never before (would hide P0)"
        );

        let (local, _tier) = seeded_tier("crc-0092-p21");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0092-p21-r");
        let remote = RemoteTier::new(&remote_root);
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::Uploaded
        );
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::AlreadyPresent,
            "identical resume still no-ops after byte-equal"
        );
        let bytes = std::fs::read(&seg).unwrap();
        let dest = remote_root.join(RemoteTier::segment_name(&bytes));
        let mut have = std::fs::read(&dest).unwrap();
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&dest, &have).unwrap();
        let msg = remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap_err()
            .to_string();
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "same-length XOR must still fail CRC first, not byte-equal; got {msg}"
        );
        assert!(
            !msg.to_ascii_lowercase().contains("read-back differs"),
            "byte-equal must not steal the P0 CRC tooth; got {msg}"
        );
    }

    /// RFC-0092 P2.2: put `crc_match_ok` + byte-equal is not a CRC32C
    /// collision theorem.
    #[test]
    fn history_put_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0093 P0: production `put_segment` ships `.bloom`; XOR one
    /// payload byte of the remote sidecar (length intact, `.hist` intact).
    /// Re-put is crc mismatch. AS-IS would no-op the sidecar.
    /// `crc_mismatch_on_live_history_put_is_not_ok` is the `.hist` tooth.
    #[test]
    fn crc_mismatch_on_live_history_sidecar_put_is_not_ok() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any same-length sidecar crc would match"
        );
        let (local, _tier) = seeded_tier("crc-0093");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0093-r");
        let remote = RemoteTier::new(&remote_root);
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::Uploaded
        );
        let bytes = std::fs::read(&seg).unwrap();
        let bloom = remote_root.join(format!("{}.bloom", RemoteTier::segment_name(&bytes)));
        let mut have = std::fs::read(&bloom).unwrap();
        assert!(have.len() >= 8 + 12, "remote sidecar must have payload");
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&bloom, &have).unwrap();
        match remote.put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg) {
            Ok(st) => {
                let _ = std::fs::remove_dir_all(&local);
                let _ = std::fs::remove_dir_all(&remote_root);
                panic!("AS-IS hole: {st:?} after same-length sidecar CRC lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&local);
                let _ = std::fs::remove_dir_all(&remote_root);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on put_sidecar crc_match_ok, not a length parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0093 P1.1: same same-length sidecar lie as P0; the collision
    /// error names the `.bloom` object. `.hist` tooth is RFC-0092, not this.
    #[test]
    fn crc_mismatch_on_live_history_sidecar_put_names_the_object() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        assert!(
            crate::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any same-length sidecar crc would match"
        );
        let (local, _tier) = seeded_tier("crc-0093-p11");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0093-p11-r");
        let remote = RemoteTier::new(&remote_root);
        remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap();
        let bytes = std::fs::read(&seg).unwrap();
        let bloom_name = format!("{}.bloom", RemoteTier::segment_name(&bytes));
        let bloom = remote_root.join(&bloom_name);
        let mut have = std::fs::read(&bloom).unwrap();
        assert!(have.len() >= 8 + 12, "remote sidecar must have payload");
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&bloom, &have).unwrap();
        let err = remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap_err();
        let msg = err.to_string();
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "must fail on put_sidecar crc_match_ok, not a length parse; got {msg}"
        );
        assert!(
            msg.contains(&bloom_name),
            "collision must name {bloom_name}, got {msg}"
        );
    }

    /// RFC-0093 P1.2: sidecar resume also requires byte-equal after
    /// `crc_match_ok`. Same-length XOR still fails CRC first (P0 tooth).
    /// Identical re-put is Ok. Byte-equal first would hide the AS-IS CRC
    /// tooth. `.hist` byte-equal is RFC-0092 P2.1, not this tooth.
    #[test]
    fn history_sidecar_put_resume_requires_byte_equal_after_crc() {
        assert!(!crate::wal::crc::crc_match_ok(1, 2));
        let hist = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/history.rs"),
        )
        .expect("history.rs");
        let sidecar_fn = hist.find("fn put_sidecar").expect("put_sidecar must exist");
        let rest = &hist[sidecar_fn..];
        let crc_at = rest
            .find("crc_match_ok(crc32c(&have), crc32c(&bytes))")
            .expect("put_sidecar CRC identity");
        let bytes_at = rest[crc_at..]
            .find("if have != bytes")
            .expect("byte-equal must follow crc_match_ok in put_sidecar");
        assert!(
            bytes_at > 0,
            "byte-equal after CRC, never before (would hide P0)"
        );

        let (local, _tier) = seeded_tier("crc-0093-p12");
        let seg = only_segment_path(&local);
        let remote_root = temp_root("crc-0093-p12-r");
        let remote = RemoteTier::new(&remote_root);
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::Uploaded
        );
        assert_eq!(
            remote
                .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::AlreadyPresent,
            "identical resume still no-ops after sidecar byte-equal"
        );
        let bytes = std::fs::read(&seg).unwrap();
        let bloom = remote_root.join(format!("{}.bloom", RemoteTier::segment_name(&bytes)));
        let mut have = std::fs::read(&bloom).unwrap();
        let mid = have.len() / 2;
        have[mid] ^= 0xff;
        std::fs::write(&bloom, &have).unwrap();
        let msg = remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap_err()
            .to_string();
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
        assert!(
            msg.to_ascii_lowercase().contains("crc mismatch"),
            "same-length XOR must still fail CRC first, not byte-equal; got {msg}"
        );
        assert!(
            !msg.to_ascii_lowercase().contains("read-back differs"),
            "byte-equal must not steal the P0 CRC tooth; got {msg}"
        );
    }

    /// RFC-0093 P2.1: sidecar put `crc_match_ok` + byte-equal is not a
    /// CRC32C collision theorem.
    #[test]
    fn history_sidecar_put_crc_collision_axiom_remains() {
        assert!(!crate::wal::crc::crc_collision_admitted());
        assert!(
            crate::wal::crc::crc_collision_admitted_as_is(),
            "AS-IS dente: matching CRC looks collision-free"
        );
        assert!(
            crate::wal::crc::crc_match_ok(1, 1),
            "equal u32s still match; that is not R-crc"
        );
        let residuals = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/formal/residuals.json");
        let text = std::fs::read_to_string(&residuals).expect("residuals.json");
        assert!(
            text.contains("\"id\": \"R-crc\""),
            "R-crc must stay in the residual catalog"
        );
        assert!(
            text.contains("\"R-crc\""),
            "never_floor must still list R-crc"
        );
    }

    /// RFC-0093 P2.2: this RFC does not fail-close the bloom prune.
    /// Trailer lie still walks (`sidecar_may_affect`); scrub stays
    /// fail-closed. Ownership of prune remains RFC-0088 / RFC-0091.
    #[test]
    fn history_sidecar_put_prune_stays_fail_open() {
        let hist = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/history.rs"),
        )
        .expect("history.rs");
        assert!(
            hist.contains("crc_mismatch_on_live_history_bloom_sidecar_may_affect_still_walks"),
            "prune fail-open stays RFC-0088 P1.1"
        );
        assert!(
            hist.contains("return true; // corrupt sidecar"),
            "mismatch must keep the fail-open walk, not prune"
        );

        let (root, _tier) = seeded_tier("crc-0093-p22");
        let path = only_bloom_path(&root);
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"PHB1", "live sidecar magic");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let scrub = verify_bloom_sidecar(&bytes).unwrap_err().to_string();
        let walks = HistoryTier::sidecar_may_affect(&bytes, b"zzz");
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            scrub.to_ascii_lowercase().contains("crc mismatch"),
            "scrub stays fail-closed; got {scrub}"
        );
        assert!(walks, "prune stays fail-open (walk), never skip on CRC lie");
    }

    #[test]
    fn remote_manifest_generations_latest_and_walkback() {
        let (root, mut tier) = seeded_tier("mani");
        let remote = RemoteTier::new(REMOTE);
        let map = MapEnv::default();
        let m1 = tier.manifest_bytes();
        let n1 = tier.remote_generation();
        assert_eq!(
            remote.put_manifest(&map, &m1, n1).unwrap(),
            PutStatus::Uploaded
        );
        // Re-put same generation: idempotent.
        assert_eq!(
            remote.put_manifest(&map, &m1, n1).unwrap(),
            PutStatus::AlreadyPresent
        );
        assert_eq!(remote.latest_manifest(&map).unwrap(), Some(m1.clone()));
        // A second, newer generation becomes LATEST.
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v4".to_vec(), 4, 0)].into_iter(),
        )
        .unwrap();
        let m2 = tier.manifest_bytes();
        let n2 = tier.remote_generation();
        remote.put_manifest(&map, &m2, n2).unwrap();
        assert_eq!(remote.latest_manifest(&map).unwrap(), Some(m2.clone()));
        // LATEST names the older generation with a bogus CRC: do not trust
        // the name (RFC-0060 P2.12) — fall back to the newest intact gen.
        {
            let mut f = map
                .create(Path::new(REMOTE).join("LATEST").as_path())
                .unwrap();
            f.write_all(format!("MANIFEST-{n1:016}\nffffffff").as_bytes())
                .unwrap();
            f.sync_all().unwrap();
        }
        assert_eq!(
            remote.latest_manifest(&map).unwrap(),
            Some(m2.clone()),
            "CRC mismatch on LATEST must not serve the named older generation"
        );
        // Torn LATEST (garbage pointer) → walk back to newest intact gen.
        {
            let mut f = map
                .create(Path::new(REMOTE).join("LATEST").as_path())
                .unwrap();
            f.write_all(b"garbage").unwrap();
            f.sync_all().unwrap();
        }
        assert_eq!(
            remote.latest_manifest(&map).unwrap(),
            Some(m2.clone()),
            "torn LATEST falls back to the newest intact generation"
        );
        // Newest generation unreadable → previous generation still serves.
        map.remove_file(
            Path::new(REMOTE)
                .join(format!("MANIFEST-{n2:016}"))
                .as_path(),
        )
        .unwrap();
        assert_eq!(remote.latest_manifest(&map).unwrap(), Some(m1));
        // Empty remote tier.
        let empty = MapEnv::default();
        assert_eq!(remote.latest_manifest(&empty).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_put_fails_closed_and_resumes() {
        let (root, _tier) = seeded_tier("resume");
        let seg = only_segment_path(&root);
        let map = MapEnv::default();
        let faulty = FaultyEnv::new(map.clone());
        faulty.fail_create.set(true);
        let remote = RemoteTier::new(REMOTE);
        assert!(remote
            .put_segment(&faulty, &crate::env::StdEnv, &seg)
            .is_err());
        assert!(
            remote_objects(&map).iter().all(|n| !n.starts_with("seg-")),
            "failed upload leaves no partial object"
        );
        faulty.fail_create.set(false);
        assert_eq!(
            remote
                .put_segment(&faulty, &crate::env::StdEnv, &seg)
                .unwrap(),
            PutStatus::Uploaded,
            "retry after the fault clears resumes and completes"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_read_segment_round_trips_records() {
        let (root, _tier) = seeded_tier("rt");
        let seg = only_segment_path(&root);
        let bytes = std::fs::read(&seg).unwrap();
        let remote = RemoteTier::new(REMOTE);
        let map = MapEnv::default();
        remote.put_segment(&map, &crate::env::StdEnv, &seg).unwrap();
        let back = remote
            .read_segment(&map, &RemoteTier::segment_name(&bytes))
            .unwrap();
        assert_eq!(back, bytes);
        let records = walk_segment_records(&back).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].key.as_slice(), b"k");
        assert_eq!(records[0].val.as_slice(), b"v1");
        assert_eq!(records[0].seq, 1);
        assert_eq!(records[2].seq, 3);
        assert!(walk_segment_records(b"").unwrap().is_empty());
        assert!(walk_segment_records(&bytes[..bytes.len() - 1]).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// RFC-0060 P2.11: remote-tier CRC walk names a flipped segment.
    #[test]
    fn remote_verify_flags_corrupt_segment() {
        let local = temp_root("rv-l");
        let remote_root = temp_root("rv-r");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &local).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v1".to_vec(), 1u64, 0u8)].into_iter(),
        )
        .unwrap();
        let seg = only_segment_path(&local);
        let remote = RemoteTier::new(&remote_root);
        remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap();
        let man = std::fs::read(local.join("history").join("MANIFEST")).unwrap();
        remote
            .put_manifest(&crate::env::StdEnv, &man, tier.remote_generation())
            .unwrap();
        let clean = remote.verify(&crate::env::StdEnv).unwrap();
        assert!(
            clean.is_clean(),
            "fresh remote must be clean: {} {:?}",
            clean.summary_line(),
            clean.failures
        );
        assert!(clean.segments >= 1);
        let obj = std::fs::read_dir(&remote_root)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "hist"))
            .expect("remote hist");
        let mut bytes = std::fs::read(&obj).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        std::fs::write(&obj, bytes).unwrap();
        let dirty = remote.verify(&crate::env::StdEnv).unwrap();
        assert!(!dirty.is_clean(), "flipped remote hist must fail");
        assert!(dirty.errors >= 1);
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
    }

    /// RFC-0060 P2.12: `archive verify` names a LATEST CRC mismatch.
    #[test]
    fn remote_verify_flags_corrupt_latest() {
        let local = temp_root("rv-latest-l");
        let remote_root = temp_root("rv-latest-r");
        let mut tier = HistoryTier::open(&crate::env::StdEnv, &local).unwrap();
        tier.archive_stream(
            &crate::env::StdEnv,
            vec![(b"k".to_vec(), b"v1".to_vec(), 1u64, 0u8)].into_iter(),
        )
        .unwrap();
        let seg = only_segment_path(&local);
        let remote = RemoteTier::new(&remote_root);
        remote
            .put_segment(&crate::env::StdEnv, &crate::env::StdEnv, &seg)
            .unwrap();
        let man = std::fs::read(local.join("history").join("MANIFEST")).unwrap();
        remote
            .put_manifest(&crate::env::StdEnv, &man, tier.remote_generation())
            .unwrap();
        let latest = remote_root.join("LATEST");
        let body = std::fs::read_to_string(&latest).unwrap();
        let name = body.split('\n').next().unwrap();
        std::fs::write(&latest, format!("{name}\nffffffff")).unwrap();
        let r = remote.verify(&crate::env::StdEnv).unwrap();
        assert!(!r.is_clean(), "LATEST crc mismatch must fail verify");
        assert!(
            r.failures
                .iter()
                .any(|(f, m)| f == "LATEST" && m.contains("crc")),
            "must name LATEST crc mismatch, got {:?}",
            r.failures
        );
        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&remote_root);
    }
}
