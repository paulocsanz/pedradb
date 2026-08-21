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
use std::io::Write;
use std::path::{Path, PathBuf};

/// One archived segment (manifest entry; file at `history/seg-{id:08}.hist`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegmentMeta {
    pub id: u64,
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
        let mut b = Vec::with_capacity(28 + self.segs.len() * 32);
        b.extend_from_slice(b"PHST");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&self.next_id.to_le_bytes());
        b.extend_from_slice(&self.archive_floor.to_le_bytes());
        b.extend_from_slice(&(self.segs.len() as u32).to_le_bytes());
        for s in &self.segs {
            b.extend_from_slice(&s.id.to_le_bytes());
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
        if u32::from_le_bytes(buf[4..8].try_into().unwrap()) != 1 {
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
            if off + 32 > body_len {
                return Err(bad());
            }
            let g = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
            segs.push_back(SegmentMeta {
                id: g(off),
                from_seq: g(off + 8),
                through_seq: g(off + 16),
                bytes: g(off + 24),
            });
            off += 32;
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
        self.manifest.segs.push_back(SegmentMeta { id, from_seq: from, through_seq: through, bytes });
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
    /// Returns the archive floor after enforcement (monotonic).
    pub(crate) fn enforce_cap<E: Env>(
        &mut self,
        env: &E,
        pin_floor: Option<u64>,
        cap_bytes: u64,
    ) -> Result<u64> {
        if cap_bytes == 0 {
            return Ok(self.manifest.archive_floor);
        }
        let mut total: u64 = self.manifest.segs.iter().map(|s| s.bytes).sum();
        let dir = self.root.join("history");
        while total > cap_bytes {
            let Some(front) = self.manifest.segs.front().copied() else { break };
            // A pin at/below the segment's top holds it — stop (fail-closed
            // toward keeping history, never toward dropping pinned data).
            if let Some(pin) = pin_floor {
                if pin <= front.through_seq {
                    break;
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
    pub(crate) fn bytes(&self) -> u64 {
        self.manifest.segs.iter().map(|s| s.bytes).sum()
    }
}
