//! Durable append-only stream over PedraDB (RFC-0010 P2.4).
//!
//! JetStream-class **need**: ordered durable messages, consumer offset, retain
//! by subject. Not NATS protocol — library API only.
//!
//! Layout:
//! - `s/{stream}/meta` → next seq u64
//! - `s/{stream}/m/{seq:020}` → payload
//! - `s/{stream}/c/{consumer}` → last delivered seq u64

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use pedradb_core::{Db, OpenOptions, Result as CoreResult};
use thiserror::Error;

/// Stream errors.
#[derive(Debug, Error)]
pub enum StreamError {
    /// Engine.
    #[error("pedradb: {0}")]
    Core(#[from] pedradb_core::CoreError),
    /// Logic.
    #[error("{0}")]
    Msg(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, StreamError>;

/// One published message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Monotonic sequence (1-based).
    pub seq: u64,
    /// Payload bytes.
    pub data: Vec<u8>,
}

/// Durable stream handle.
pub struct Stream {
    db: Db,
    name: String,
}

impl Stream {
    /// Open or create stream `name` in `path`.
    ///
    /// # Errors
    /// PedraDB open / meta init.
    pub fn open(path: impl AsRef<std::path::Path>, name: &str) -> Result<Self> {
        if name.is_empty() || name.contains('/') {
            return Err(StreamError::Msg("bad stream name".into()));
        }
        let db = Db::open_with(
            path,
            OpenOptions {
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )?;
        let mut s = Self {
            db,
            name: name.to_string(),
        };
        if s.db.get(&s.meta_key()).is_none() {
            s.db.put(s.meta_key(), 0u64.to_le_bytes())?;
        }
        Ok(s)
    }

    fn meta_key(&self) -> Vec<u8> {
        format!("s/{}/meta", self.name).into_bytes()
    }

    fn msg_key(&self, seq: u64) -> Vec<u8> {
        format!("s/{}/m/{seq:020}", self.name).into_bytes()
    }

    fn consumer_key(&self, consumer: &str) -> Vec<u8> {
        format!("s/{}/c/{consumer}", self.name).into_bytes()
    }

    /// Last published sequence (0 if empty).
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        self.db
            .get(&self.meta_key())
            .and_then(|b| {
                if b.len() >= 8 {
                    Some(u64::from_le_bytes(b[..8].try_into().ok()?))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    /// Append a message; returns assigned sequence. Durable before return.
    ///
    /// # Errors
    /// I/O.
    pub fn publish(&mut self, data: impl AsRef<[u8]>) -> Result<u64> {
        let seq = self.last_seq() + 1;
        let msg_k = self.msg_key(seq);
        let meta_k = self.meta_key();
        let mut tx = self.db.begin();
        tx.put(msg_k, data.as_ref())?;
        tx.put(meta_k, seq.to_le_bytes())?;
        tx.commit()?;
        Ok(seq)
    }

    /// Fetch message by sequence.
    #[must_use]
    pub fn get(&self, seq: u64) -> Option<Message> {
        let data = self.db.get(&self.msg_key(seq))?;
        Some(Message {
            seq,
            data: data.to_vec(),
        })
    }

    /// Read next message for `consumer` (advances cursor after read).
    ///
    /// # Errors
    /// I/O when updating cursor.
    pub fn next(&mut self, consumer: &str) -> Result<Option<Message>> {
        if consumer.is_empty() || consumer.contains('/') {
            return Err(StreamError::Msg("bad consumer name".into()));
        }
        let last = self
            .db
            .get(&self.consumer_key(consumer))
            .and_then(|b| {
                if b.len() >= 8 {
                    Some(u64::from_le_bytes(b[..8].try_into().ok()?))
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let want = last + 1;
        let Some(msg) = self.get(want) else {
            return Ok(None);
        };
        self.db
            .put(self.consumer_key(consumer), want.to_le_bytes())?;
        Ok(Some(msg))
    }

    /// Consumer cursor (last delivered seq).
    #[must_use]
    pub fn consumer_seq(&self, consumer: &str) -> u64 {
        self.db
            .get(&self.consumer_key(consumer))
            .and_then(|b| {
                if b.len() >= 8 {
                    Some(u64::from_le_bytes(b[..8].try_into().ok()?))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    /// Close.
    ///
    /// # Errors
    /// WAL close.
    pub fn close(self) -> CoreResult<()> {
        self.db.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedradb-stream-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn publish_consume_durable() {
        let dir = temp();
        {
            let mut s = Stream::open(&dir, "events").unwrap();
            assert_eq!(s.publish(b"a").unwrap(), 1);
            assert_eq!(s.publish(b"b").unwrap(), 2);
            let m = s.next("c1").unwrap().unwrap();
            assert_eq!(m.seq, 1);
            assert_eq!(m.data, b"a");
            let m = s.next("c1").unwrap().unwrap();
            assert_eq!(m.seq, 2);
            assert!(s.next("c1").unwrap().is_none());
            s.close().unwrap();
        }
        // Reopen: consumer cursor and messages survive.
        let mut s = Stream::open(&dir, "events").unwrap();
        assert_eq!(s.last_seq(), 2);
        assert_eq!(s.consumer_seq("c1"), 2);
        assert_eq!(s.get(1).unwrap().data, b"a");
        // Second consumer from start.
        let m = s.next("c2").unwrap().unwrap();
        assert_eq!(m.seq, 1);
        s.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
