//! Persist Raft hard state + log on disk (RFC-0012 P0.2 / RFC-0015 P1.2).
//!
//! On-disk integrity (F9): trailing CRC32C on hard state and log; decode refuses
//! untrusted `with_capacity` counts larger than remaining bytes (SST F2 class).
//!
//! Durable I/O goes through [`pedradb_core::Env`] so FailingEnv can inject mid-write
//! / sync failures. Path-only APIs default to [`StdEnv`].

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pedradb_core::{BatchOp, Env, EnvFile, StdEnv};

use crate::{HardState, RaftError, RaftLogEntry, Result};

const HARD_NAME: &str = "RAFT_HARD";
const LOG_NAME: &str = "RAFT_LOG";
const COMMIT_NAME: &str = "RAFT_COMMIT";
const MAGIC: &[u8; 4] = b"PRFT";
/// Format version 2: payload + trailing CRC32C (v1 had no checksum).
const VERSION: u32 = 2;
/// Minimum encoded size of a log entry with zero ops: index + term + nop count.
const MIN_ENTRY_BYTES: usize = 8 + 8 + 4;
/// Minimum encoded op: tag + empty-key delete = 1 + 4.
const MIN_OP_BYTES: usize = 1 + 4;

/// Directory holding Raft meta next to the PedraDB data dir.
#[must_use]
pub fn raft_meta_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("raft-meta")
}

/// Load hard state; default if missing. Uses [`StdEnv`].
///
/// # Errors
/// I/O or corrupt file.
pub fn load_hard(meta_dir: &Path) -> Result<HardState> {
    load_hard_on(&StdEnv, meta_dir)
}

/// Load hard state via [`Env`].
///
/// # Errors
/// I/O or corrupt file.
pub fn load_hard_on<E: Env>(env: &E, meta_dir: &Path) -> Result<HardState> {
    let path = meta_dir.join(HARD_NAME);
    if !env.exists(&path) {
        return Ok(HardState::default());
    }
    let mut f = env.open_read(&path).map_err(io_err)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(io_err)?;
    // magic+ver+term+has_vote + CRC32C
    if buf.len() < 4 + 4 + 8 + 1 + 4 {
        return Err(RaftError::Persist("hard state too short".into()));
    }
    let (payload, crc_raw) = buf.split_at(buf.len() - 4);
    let expect = crc32c::crc32c(payload);
    let got = u32::from_le_bytes(crc_raw.try_into().unwrap());
    if !pedradb_core::wal::crc::crc_match_ok(got, expect) {
        return Err(RaftError::Persist("hard state CRC mismatch".into()));
    }
    if &payload[0..4] != MAGIC {
        return Err(RaftError::Persist("hard magic".into()));
    }
    let ver = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    if ver != VERSION {
        return Err(RaftError::Persist(format!("hard version {ver}")));
    }
    let term = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    let has_vote = payload[16];
    let voted_for = if has_vote == 1 {
        if payload.len() < 25 {
            return Err(RaftError::Persist("hard vote truncated".into()));
        }
        Some(u64::from_le_bytes(payload[17..25].try_into().unwrap()))
    } else {
        None
    };
    Ok(HardState {
        current_term: term,
        voted_for,
    })
}

/// Persist hard state (atomic tmp+rename). Uses [`StdEnv`].
///
/// # Errors
/// I/O.
pub fn store_hard(meta_dir: &Path, hard: &HardState) -> Result<()> {
    store_hard_on(&StdEnv, meta_dir, hard)
}

/// Persist hard state via [`Env`] (atomic tmp+rename + dir sync).
///
/// # Errors
/// I/O.
pub fn store_hard_on<E: Env>(env: &E, meta_dir: &Path, hard: &HardState) -> Result<()> {
    env.create_dir_all(meta_dir).map_err(io_err)?;
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_le_bytes());
    body.extend_from_slice(&hard.current_term.to_le_bytes());
    match hard.voted_for {
        Some(v) => {
            body.push(1);
            body.extend_from_slice(&v.to_le_bytes());
        }
        None => body.push(0),
    }
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    atomic_write_on(env, &meta_dir.join(HARD_NAME), &body)
}

/// Load durable commit index (0 if missing). Uses [`StdEnv`].
///
/// # Errors
/// I/O or corrupt.
pub fn load_commit(meta_dir: &Path) -> Result<u64> {
    load_commit_on(&StdEnv, meta_dir)
}

/// Load commit index via [`Env`].
///
/// # Errors
/// I/O or corrupt.
pub fn load_commit_on<E: Env>(env: &E, meta_dir: &Path) -> Result<u64> {
    let path = meta_dir.join(COMMIT_NAME);
    if !env.exists(&path) {
        return Ok(0);
    }
    let mut f = env.open_read(&path).map_err(io_err)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(io_err)?;
    // magic + ver + commit_u64 + crc
    if buf.len() < 4 + 4 + 8 + 4 {
        return Err(RaftError::Persist("commit too short".into()));
    }
    let (payload, crc_raw) = buf.split_at(buf.len() - 4);
    let expect = crc32c::crc32c(payload);
    let got = u32::from_le_bytes(crc_raw.try_into().unwrap());
    if !pedradb_core::wal::crc::crc_match_ok(got, expect) {
        return Err(RaftError::Persist("commit CRC mismatch".into()));
    }
    if &payload[0..4] != MAGIC {
        return Err(RaftError::Persist("commit magic".into()));
    }
    let ver = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    if ver != VERSION {
        return Err(RaftError::Persist(format!("commit version {ver}")));
    }
    Ok(u64::from_le_bytes(payload[8..16].try_into().unwrap()))
}

/// Persist commit index (atomic tmp+rename + CRC). Uses [`StdEnv`].
///
/// # Errors
/// I/O.
pub fn store_commit(meta_dir: &Path, commit_index: u64) -> Result<()> {
    store_commit_on(&StdEnv, meta_dir, commit_index)
}

/// Persist commit index via [`Env`].
///
/// # Errors
/// I/O.
pub fn store_commit_on<E: Env>(env: &E, meta_dir: &Path, commit_index: u64) -> Result<()> {
    env.create_dir_all(meta_dir).map_err(io_err)?;
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_le_bytes());
    body.extend_from_slice(&commit_index.to_le_bytes());
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    atomic_write_on(env, &meta_dir.join(COMMIT_NAME), &body)
}

/// Load full raft log. Uses [`StdEnv`].
///
/// # Errors
/// I/O or corrupt.
pub fn load_log(meta_dir: &Path) -> Result<Vec<RaftLogEntry>> {
    load_log_on(&StdEnv, meta_dir)
}

/// Load full raft log via [`Env`].
///
/// # Errors
/// I/O or corrupt.
pub fn load_log_on<E: Env>(env: &E, meta_dir: &Path) -> Result<Vec<RaftLogEntry>> {
    let path = meta_dir.join(LOG_NAME);
    if !env.exists(&path) {
        return Ok(Vec::new());
    }
    let mut f = env.open_read(&path).map_err(io_err)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(io_err)?;
    decode_log(&buf)
}

/// Store full raft log. Uses [`StdEnv`].
///
/// # Errors
/// I/O.
pub fn store_log(meta_dir: &Path, log: &[RaftLogEntry]) -> Result<()> {
    store_log_on(&StdEnv, meta_dir, log)
}

/// Store full raft log via [`Env`].
///
/// # Errors
/// I/O.
pub fn store_log_on<E: Env>(env: &E, meta_dir: &Path, log: &[RaftLogEntry]) -> Result<()> {
    env.create_dir_all(meta_dir).map_err(io_err)?;
    let body = encode_log(log)?;
    atomic_write_on(env, &meta_dir.join(LOG_NAME), &body)
}

fn atomic_write_on<E: Env>(env: &E, path: &Path, body: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = env.create(&tmp).map_err(io_err)?;
        f.write_all(body).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }
    env.rename(&tmp, path).map_err(io_err)?;
    // F17 / RFC-0015: fsync parent dir so the rename is durable; errors surface.
    if let Some(parent) = path.parent() {
        env.sync_dir(parent).map_err(io_err)?;
    }
    Ok(())
}

fn io_err(e: std::io::Error) -> RaftError {
    RaftError::Persist(e.to_string())
}

fn encode_log(log: &[RaftLogEntry]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_le_bytes());
    body.extend_from_slice(&(log.len() as u64).to_le_bytes());
    for e in log {
        body.extend_from_slice(&e.index.to_le_bytes());
        body.extend_from_slice(&e.term.to_le_bytes());
        body.extend_from_slice(&(e.ops.len() as u32).to_le_bytes());
        for op in &e.ops {
            encode_op(&mut body, op)?;
        }
    }
    let crc = crc32c::crc32c(&body);
    body.extend_from_slice(&crc.to_le_bytes());
    Ok(body)
}

fn encode_op(body: &mut Vec<u8>, op: &BatchOp) -> Result<()> {
    match op {
        BatchOp::Put { key, value } => {
            body.push(1);
            put_bytes(body, key);
            put_bytes(body, value);
        }
        BatchOp::Delete { key } => {
            body.push(2);
            put_bytes(body, key);
        }
        BatchOp::DeleteRange { start, end } => {
            body.push(3);
            put_bytes(body, start);
            put_bytes(body, end);
        }
    }
    Ok(())
}

fn put_bytes(body: &mut Vec<u8>, b: &[u8]) {
    body.extend_from_slice(&(b.len() as u32).to_le_bytes());
    body.extend_from_slice(b);
}

fn decode_log(buf: &[u8]) -> Result<Vec<RaftLogEntry>> {
    // header 16 + CRC 4
    if buf.len() < 4 + 4 + 8 + 4 {
        return Err(RaftError::Persist("log too short".into()));
    }
    let (payload, crc_raw) = buf.split_at(buf.len() - 4);
    let expect = crc32c::crc32c(payload);
    let got = u32::from_le_bytes(crc_raw.try_into().unwrap());
    if !pedradb_core::wal::crc::crc_match_ok(got, expect) {
        return Err(RaftError::Persist("log CRC mismatch".into()));
    }
    if &payload[0..4] != MAGIC {
        return Err(RaftError::Persist("log magic".into()));
    }
    let ver = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    if ver != VERSION {
        return Err(RaftError::Persist(format!("log version {ver}")));
    }
    let n_raw = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    let rem = payload.len().saturating_sub(16);
    // F9: refuse multi-EiB with_capacity from a bit-flipped count (F2 class).
    let max_n = rem / MIN_ENTRY_BYTES;
    if n_raw as usize > max_n || n_raw > usize::MAX as u64 {
        return Err(RaftError::Persist(format!(
            "log entry count {n_raw} exceeds remaining {rem} bytes"
        )));
    }
    let n = n_raw as usize;
    let mut off = 16;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let index = read_u64(payload, &mut off)?;
        let term = read_u64(payload, &mut off)?;
        let nop_raw = read_u32(payload, &mut off)?;
        let rem_ops = payload.len().saturating_sub(off);
        let max_ops = rem_ops / MIN_OP_BYTES;
        if nop_raw as usize > max_ops {
            return Err(RaftError::Persist(format!(
                "log op count {nop_raw} exceeds remaining {rem_ops} bytes"
            )));
        }
        let nop = nop_raw as usize;
        let mut ops = Vec::with_capacity(nop);
        for _ in 0..nop {
            ops.push(decode_op(payload, &mut off)?);
        }
        out.push(RaftLogEntry { index, term, ops });
    }
    if off != payload.len() {
        return Err(RaftError::Persist(format!(
            "log trailing garbage: off={off} len={}",
            payload.len()
        )));
    }
    Ok(out)
}

fn decode_op(buf: &[u8], off: &mut usize) -> Result<BatchOp> {
    let tag = read_u8(buf, off)?;
    match tag {
        1 => {
            let key = read_bytes(buf, off)?;
            let value = read_bytes(buf, off)?;
            Ok(BatchOp::Put {
                key: bytes::Bytes::from(key),
                value: bytes::Bytes::from(value),
            })
        }
        2 => {
            let key = read_bytes(buf, off)?;
            Ok(BatchOp::Delete {
                key: bytes::Bytes::from(key),
            })
        }
        3 => {
            let start = read_bytes(buf, off)?;
            let end = read_bytes(buf, off)?;
            Ok(BatchOp::DeleteRange {
                start: bytes::Bytes::from(start),
                end: bytes::Bytes::from(end),
            })
        }
        _ => Err(RaftError::Persist(format!("bad op tag {tag}"))),
    }
}

fn read_u8(buf: &[u8], off: &mut usize) -> Result<u8> {
    if *off >= buf.len() {
        return Err(RaftError::Persist("eof u8".into()));
    }
    let v = buf[*off];
    *off += 1;
    Ok(v)
}

fn read_u32(buf: &[u8], off: &mut usize) -> Result<u32> {
    if *off + 4 > buf.len() {
        return Err(RaftError::Persist("eof u32".into()));
    }
    let v = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}

fn read_u64(buf: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > buf.len() {
        return Err(RaftError::Persist("eof u64".into()));
    }
    let v = u64::from_le_bytes(buf[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

fn read_bytes(buf: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    let n = read_u32(buf, off)? as usize;
    if *off + n > buf.len() {
        return Err(RaftError::Persist("eof bytes".into()));
    }
    let v = buf[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-raft-persist-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hard_and_log_round_trip() {
        let dir = temp();
        let hard = HardState {
            current_term: 7,
            voted_for: Some(2),
        };
        store_hard(&dir, &hard).unwrap();
        assert_eq!(load_hard(&dir).unwrap().current_term, 7);
        assert_eq!(load_hard(&dir).unwrap().voted_for, Some(2));

        let log = vec![RaftLogEntry {
            index: 1,
            term: 7,
            ops: vec![
                BatchOp::put(b"a", b"1"),
                BatchOp::delete(b"b"),
                BatchOp::delete_range(b"c", b"e"),
            ],
        }];
        store_log(&dir, &log).unwrap();
        let loaded = load_log(&dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].index, 1);
        assert_eq!(loaded[0].ops.len(), 3);
        assert!(matches!(
            &loaded[0].ops[2],
            BatchOp::DeleteRange { start, end }
                if start.as_ref() == b"c" && end.as_ref() == b"e"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F9: huge entry count must fail-stop, not allocate multi-EiB.
    #[test]
    fn decode_log_rejects_huge_entry_count() {
        // Valid magic/version but count claims 2^40 entries with no payload.
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&0x0000_0100_0000_0000u64.to_le_bytes());
        let crc = crc32c::crc32c(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        let err = decode_log(&buf).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exceeds remaining") || msg.contains("CRC"),
            "expected count bound error, got {msg}"
        );
    }

    /// F9: bit-flip on stored log must not silently load a shorter log.
    #[test]
    fn log_byte_flip_fail_stops() {
        let dir = temp();
        let log = vec![
            RaftLogEntry {
                index: 1,
                term: 1,
                ops: vec![BatchOp::put(b"k", b"v")],
            },
            RaftLogEntry {
                index: 2,
                term: 1,
                ops: vec![BatchOp::put(b"k2", b"v2")],
            },
        ];
        store_log(&dir, &log).unwrap();
        let path = dir.join(LOG_NAME);
        let mut raw = std::fs::read(&path).unwrap();
        // Flip a mid-file byte (entry count / payload region).
        let i = raw.len() / 2;
        raw[i] ^= 0x01;
        std::fs::write(&path, &raw).unwrap();
        let err = load_log(&dir).unwrap_err();
        assert!(
            err.to_string().contains("CRC") || err.to_string().contains("log"),
            "expected integrity error, got {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_byte_flip_fail_stops() {
        let dir = temp();
        store_hard(
            &dir,
            &HardState {
                current_term: 3,
                voted_for: Some(1),
            },
        )
        .unwrap();
        let path = dir.join(HARD_NAME);
        let mut raw = std::fs::read(&path).unwrap();
        raw[10] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        assert!(load_hard(&dir)
            .unwrap_err()
            .to_string()
            .to_ascii_lowercase()
            .contains("crc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0090 P0: production `store_hard` writes `RAFT_HARD`; XOR only
    /// the trailer CRC (magic/term/vote intact). Load is crc mismatch.
    /// AS-IS would return the stored term. `hard_byte_flip_fail_stops`
    /// (payload byte 10) is not this tooth.
    #[test]
    fn crc_mismatch_on_live_raft_hard_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any hard-state crc would match"
        );
        let dir = temp();
        store_hard(
            &dir,
            &HardState {
                current_term: 3,
                voted_for: Some(1),
            },
        )
        .unwrap();
        let path = dir.join(HARD_NAME);
        let mut raw = std::fs::read(&path).unwrap();
        assert!(
            raw.len() >= 4 + 4 + 8 + 1 + 4,
            "RAFT_HARD must have payload + trailer"
        );
        assert_eq!(&raw[0..4], MAGIC, "live hard-state magic");
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        match load_hard(&dir) {
            Ok(h) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!(
                    "AS-IS hole: served term {} after CRC trailer lie",
                    h.current_term
                );
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a magic/term parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0090 P1.1: production `store_commit` writes `RAFT_COMMIT`; XOR
    /// only the trailer CRC (magic/index intact). Load is crc mismatch.
    /// AS-IS would return the stored commit index.
    #[test]
    fn crc_mismatch_on_live_raft_commit_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any commit crc would match"
        );
        let dir = temp();
        store_commit(&dir, 42).unwrap();
        let path = dir.join(COMMIT_NAME);
        let mut raw = std::fs::read(&path).unwrap();
        assert!(
            raw.len() >= 4 + 4 + 8 + 4,
            "RAFT_COMMIT must have payload + trailer"
        );
        assert_eq!(&raw[0..4], MAGIC, "live commit magic");
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        match load_commit(&dir) {
            Ok(n) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!("AS-IS hole: served commit {n} after CRC trailer lie");
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a magic/index parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0090 P1.2: production `store_log` writes `RAFT_LOG`; XOR only
    /// the trailer CRC (magic/entries intact). Load is crc mismatch.
    /// AS-IS would return the stored entries. `log_byte_flip_fail_stops`
    /// (mid-file payload) is not this tooth.
    #[test]
    fn crc_mismatch_on_live_raft_log_is_not_ok() {
        assert!(!pedradb_core::wal::crc::crc_match_ok(1, 2));
        assert!(
            pedradb_core::wal::crc::crc_match_ok_as_is(1, 2),
            "AS-IS dente: any log crc would match"
        );
        let dir = temp();
        let log = vec![RaftLogEntry {
            index: 1,
            term: 1,
            ops: vec![BatchOp::put(b"k", b"v")],
        }];
        store_log(&dir, &log).unwrap();
        let path = dir.join(LOG_NAME);
        let mut raw = std::fs::read(&path).unwrap();
        assert!(
            raw.len() >= 4 + 4 + 8 + 4,
            "RAFT_LOG must have payload + trailer"
        );
        assert_eq!(&raw[0..4], MAGIC, "live log magic");
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        std::fs::write(&path, &raw).unwrap();
        match load_log(&dir) {
            Ok(got) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!(
                    "AS-IS hole: served {} log entries after CRC trailer lie",
                    got.len()
                );
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                let msg = e.to_string();
                assert!(
                    msg.to_ascii_lowercase().contains("crc mismatch"),
                    "must fail on crc_match_ok, not a magic/count parse; got {msg}"
                );
            }
        }
    }

    /// RFC-0015 P1.4: Env-backed store surfaces injected create/sync failure.
    #[test]
    fn store_hard_on_failing_env_nth_op() {
        use pedradb_sim::FailingEnv;

        let dir = temp();
        // fail_after(0): first fallible op (create_dir_all or create) fails.
        let env = FailingEnv::fail_after(0);
        let err = store_hard_on(
            &env,
            &dir,
            &HardState {
                current_term: 1,
                voted_for: None,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("injected") || err.to_string().contains("Persist"),
            "expected injected I/O, got {err}"
        );
        assert!(env.tripped());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_hard_on_sync_fail_surfaces() {
        use pedradb_sim::{FailingEnv, FaultKind};

        let dir = temp();
        // Writes land; first sync (file sync_all in atomic_write) fails.
        let env = FailingEnv::fail_after_kind(0, FaultKind::SyncFail);
        let err = store_hard_on(
            &env,
            &dir,
            &HardState {
                current_term: 9,
                voted_for: Some(1),
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("sync") || err.to_string().contains("Persist"),
            "expected sync failure, got {err}"
        );
        assert!(env.tripped());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
