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
        b.extend_from_slice(&2u32.to_le_bytes());
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
        }
        let crc = crc32c(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        let bad = || CoreError::CorruptManifest("history manifest".into());
        if buf.len() < 32 || &buf[0..4] != b"PHST" {
            return Err(bad());
        }
        if u32::from_le_bytes(buf[4..8].try_into().unwrap()) != 2 {
            return Err(bad());
        }
        let body_len = buf.len() - 4;
        let crc = u32::from_le_bytes(buf[body_len..].try_into().unwrap());
        if crc32c(&buf[..body_len]) != crc {
            return Err(bad());
        }
        let next_id = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let archive_floor = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let n = u32::from_le_bytes(buf[24..28].try_into().unwrap()) as usize;
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
            let nlen =
                u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
            off += 4;
            if off + nlen + 24 > body_len {
                return Err(bad());
            }
            let name = String::from_utf8_lossy(&buf[off..off + nlen]).into_owned();
            off += nlen;
            let from_seq = g(&mut off)?;
            let through_seq = g(&mut off)?;
            let bytes = g(&mut off)?;
            segs.push_back(SegmentMeta { id, name, from_seq, through_seq, bytes });
        }
        Ok(Self { next_id, segs, archive_floor })
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
        let mut manifest =
            Manifest { next_id: 1, segs: VecDeque::new(), archive_floor: 0 };
        if env.exists(&dir) {
            let path = dir.join("MANIFEST");
            if env.exists(&path) {
                let mut f = env.open_read(&path)?;
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut buf)?;
                manifest = Manifest::decode(&buf)?;
            }
            let live: std::collections::HashSet<u64> =
                manifest.segs.iter().map(|s| s.id).collect();
            for name in env.read_dir_names(&dir).unwrap_or_default() {
                if let Some(id) = name
                    .strip_prefix("seg-")
                    .and_then(|s| s.strip_suffix(".hist"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    if !live.contains(&id) {
                        let _ = env.remove_file(&dir.join(name));
                    }
                }
            }
        }
        Ok(Self { root: db_root.to_path_buf(), manifest })
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
        let mut buf: Vec<u8> = Vec::new();
        for (key, val, seq, kind) in records {
            if cur_file.is_none() {
                env.create_dir_all(&dir)?;
                cur_id = self.manifest.next_id;
                self.manifest.next_id += 1;
                cur_file = Some(env.create(&dir.join(format!("seg-{cur_id:08}.hist")))?);
                (cur_n, cur_bytes, cur_from, cur_through) = (0, 0, u64::MAX, 0);
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
            if cur_n >= SEG_MAX_RECORDS {
                self.seal_segment(env, cur_file.take().unwrap(), cur_id, cur_from, cur_through, cur_bytes)?;
            }
        }
        if let Some(f) = cur_file.take() {
            self.seal_segment(env, f, cur_id, cur_from, cur_through, cur_bytes)?;
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
    ) -> Result<()> {
        f.sync_all()?;
        drop(f);
        // Content-addressed name (remote mirror object identity): read the
        // sealed bytes back once and hash them.
        let path = self.root.join("history").join(format!("seg-{id:08}.hist"));
        let mut rf = env.open_read(&path)?;
        let mut buf = Vec::with_capacity(bytes as usize);
        std::io::Read::read_to_end(&mut rf, &mut buf)?;
        let name = RemoteTier::segment_name(&buf);
        self.manifest.segs.push_back(SegmentMeta {
            id,
            name,
            from_seq: from,
            through_seq: through,
            bytes,
        });
        self.persist(env)
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
            let Some(front) = self.manifest.segs.front().cloned() else { break };
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
    #[cfg_attr(not(test), allow(dead_code))] // test + P1.2 sizing metrics
    pub(crate) fn bytes(&self) -> u64 {
        self.manifest.segs.iter().map(|s| s.bytes).sum()
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
        if crc32c(&bytes[start..off - 4]) != stored {
            return Err(bad("crc mismatch"));
        }
        out.push(HistoryRecord { key, val, seq, kind });
    }
    Ok(out)
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

    /// Content-addressed object name for segment `bytes`.
    pub fn segment_name(bytes: &[u8]) -> String {
        format!("seg-{:016x}-{:08x}.hist", bytes.len() as u64, crc32c(bytes))
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
        if remote_env.exists(&dest) {
            let mut rf = remote_env.open_read(&dest)?;
            let mut have = Vec::new();
            std::io::Read::read_to_end(&mut rf, &mut have)?;
            if have.len() == bytes.len() && crc32c(&have) == crc32c(&bytes) {
                return Ok(PutStatus::AlreadyPresent);
            }
            return Err(CoreError::CorruptHistory(format!(
                "remote name collision at {name}: read-back differs"
            )));
        }
        remote_env.create_dir_all(&self.root)?;
        {
            let mut out = remote_env.create(&dest)?;
            out.write_all(&bytes)?;
            out.sync_all()?;
        }
        remote_env.sync_dir(&self.root)?;
        Ok(PutStatus::Uploaded)
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
    pub fn put_manifest<E: Env>(
        &self,
        env: &E,
        bytes: &[u8],
        n: u64,
    ) -> Result<PutStatus> {
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
            })
            .collect())
    }

    /// Newest intact manifest generation: `LATEST` if it parses and its
    /// target decodes; otherwise the highest-numbered intact generation;
    /// `None` when the remote tier is empty.
    pub fn latest_manifest<E: Env>(&self, env: &E) -> Result<Option<Vec<u8>>> {
        let latest = self.segment_path("LATEST");
        if env.exists(&latest) {
            if let Ok(mut f) = env.open_read(&latest) {
                let mut buf = String::new();
                if f
                    .read_to_string(&mut buf)
                    .is_ok_and(|_| !buf.is_empty())
                {
                    if let Some((name, _crc)) = buf.trim_end().split_once('\n') {
                        let p = self.segment_path(name);
                        if env.exists(&p) {
                            if let Ok(mut mf) = env.open_read(&p) {
                                let mut mb = Vec::new();
                                if std::io::Read::read_to_end(&mut mf, &mut mb).is_ok()
                                    && Manifest::decode(&mb).is_ok()
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
            let buf = self.files.borrow().get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "missing")
            })?;
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
            Self { inner, fail_create: Rc::new(Cell::new(false)) }
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
        tier.archive_stream(&crate::env::StdEnv, records.into_iter()).unwrap();
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

    fn remote_objects(map: &MapEnv) -> Vec<String> {
        map.read_dir_names(Path::new("/remote")).unwrap()
    }

    const REMOTE: &str = "/remote";

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
        let objects: Vec<String> =
            remote_objects(&map).into_iter().filter(|n| n.starts_with("seg-")).collect();
        assert_eq!(objects.len(), 1, "idempotent put must not duplicate objects");
        let bytes = std::fs::read(&seg).unwrap();
        assert_eq!(objects[0], RemoteTier::segment_name(&bytes));
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

    #[test]
    fn remote_manifest_generations_latest_and_walkback() {
        let (root, mut tier) = seeded_tier("mani");
        let remote = RemoteTier::new(REMOTE);
        let map = MapEnv::default();
        let m1 = tier.manifest_bytes();
        let n1 = tier.remote_generation();
        assert_eq!(remote.put_manifest(&map, &m1, n1).unwrap(), PutStatus::Uploaded);
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
        // Torn LATEST (garbage pointer) → walk back to newest intact gen.
        {
            let mut f = map.create(Path::new(REMOTE).join("LATEST").as_path()).unwrap();
            f.write_all(b"garbage").unwrap();
            f.sync_all().unwrap();
        }
        assert_eq!(
            remote.latest_manifest(&map).unwrap(),
            Some(m2.clone()),
            "torn LATEST falls back to the newest intact generation"
        );
        // Newest generation unreadable → previous generation still serves.
        map.remove_file(Path::new(REMOTE).join(format!("MANIFEST-{n2:016}")).as_path())
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
        assert!(remote.put_segment(&faulty, &crate::env::StdEnv, &seg).is_err());
        assert!(
            remote_objects(&map).iter().all(|n| !n.starts_with("seg-")),
            "failed upload leaves no partial object"
        );
        faulty.fail_create.set(false);
        assert_eq!(
            remote.put_segment(&faulty, &crate::env::StdEnv, &seg).unwrap(),
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
}
