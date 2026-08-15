//! Durable append-only stream over PedraDB (RFC-0010 P2.4).
//!
//! JetStream-class **need**: ordered durable messages, consumer offset, retain
//! by subject. Not NATS protocol — library API only.
//!
//! Layout (F70 — length-prefixed stream name; not slash-joined):
//! - `s/` + len(name) + name + `\0M` → next seq u64
//! - `s/` + len(name) + name + `\0m` + `{seq:020}` → payload
//! - `s/` + len(name) + name + `\0c` + len(consumer) + consumer → last acked seq

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cursor_kernel;

pub use cursor_kernel::{
    ack_in_order, ack_in_order_as_is, next_seq, peek_pins_cursor, peek_pins_cursor_as_is,
};

use pedradb_core::{Db, OpenOptions, Result as CoreResult};
use thiserror::Error;

fn push_len_pref(buf: &mut Vec<u8>, part: &[u8]) {
    let n = u32::try_from(part.len()).expect("component len fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(part);
}

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
        if name.is_empty() {
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

    fn stream_ns(&self) -> Vec<u8> {
        let mut k = b"s/".to_vec();
        push_len_pref(&mut k, self.name.as_bytes());
        k
    }

    fn meta_key(&self) -> Vec<u8> {
        let mut k = self.stream_ns();
        k.push(0x00);
        k.push(b'M');
        k
    }

    fn msg_key(&self, seq: u64) -> Vec<u8> {
        let mut k = self.stream_ns();
        k.push(0x00);
        k.push(b'm');
        k.extend_from_slice(format!("{seq:020}").as_bytes());
        k
    }

    fn consumer_key(&self, consumer: &str) -> Vec<u8> {
        let mut k = self.stream_ns();
        k.push(0x00);
        k.push(b'c');
        push_len_pref(&mut k, consumer.as_bytes());
        k
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

    fn load_consumer_seq(&self, consumer: &str) -> Result<u64> {
        if consumer.is_empty() {
            return Err(StreamError::Msg("bad consumer name".into()));
        }
        Ok(self
            .db
            .get(&self.consumer_key(consumer))
            .and_then(|b| {
                if b.len() >= 8 {
                    Some(u64::from_le_bytes(b[..8].try_into().ok()?))
                } else {
                    None
                }
            })
            .unwrap_or(0))
    }

    /// Peek next message **without** advancing the cursor (at-least-once).
    ///
    /// Fold/RFC-0024: pin only after the caller applied the message ([`ack`]).
    ///
    /// # Errors
    /// Bad consumer name.
    pub fn peek(&self, consumer: &str) -> Result<Option<Message>> {
        let last = self.load_consumer_seq(consumer)?;
        debug_assert!(!cursor_kernel::peek_pins_cursor());
        Ok(self.get(cursor_kernel::next_seq(last)))
    }

    /// Persist cursor through `seq` after the caller applied that message.
    ///
    /// # Errors
    /// Bad name, I/O, or `seq` not the next expected (no holes).
    pub fn ack(&mut self, consumer: &str, seq: u64) -> Result<()> {
        let last = self.load_consumer_seq(consumer)?;
        if !cursor_kernel::ack_in_order(last, seq) {
            return Err(StreamError::Msg(format!(
                "ack {seq} out of order (cursor {last})"
            )));
        }
        if self.get(seq).is_none() {
            return Err(StreamError::Msg(format!("ack {seq}: no such message")));
        }
        self.db
            .put(self.consumer_key(consumer), seq.to_le_bytes())?;
        Ok(())
    }

    /// Read next message and **immediately** ack (at-most-once / lab convenience).
    ///
    /// Crash after this returns may skip the message. Prefer [`peek`] + [`ack`].
    ///
    /// # Errors
    /// I/O when updating cursor.
    pub fn next(&mut self, consumer: &str) -> Result<Option<Message>> {
        let Some(msg) = self.peek(consumer)? else {
            return Ok(None);
        };
        self.ack(consumer, msg.seq)?;
        Ok(Some(msg))
    }

    /// Consumer cursor (last **acked** seq).
    #[must_use]
    pub fn consumer_seq(&self, consumer: &str) -> u64 {
        self.load_consumer_seq(consumer).unwrap_or(0)
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

    /// RFC-0024 class: peek without ack; crash; same message still next.
    #[test]
    fn peek_without_ack_survives_reopen() {
        let dir = temp();
        {
            let mut s = Stream::open(&dir, "events").unwrap();
            s.publish(b"keep").unwrap();
            s.publish(b"later").unwrap();
            let m = s.peek("c1").unwrap().unwrap();
            assert_eq!(m.seq, 1);
            assert_eq!(m.data, b"keep");
            assert_eq!(s.consumer_seq("c1"), 0, "peek must not pin");
            drop(s);
        }
        let mut s = Stream::open(&dir, "events").unwrap();
        let m = s.peek("c1").unwrap().unwrap();
        assert_eq!(
            m.data, b"keep",
            "unacked peek must not skip after reopen"
        );
        s.ack("c1", 1).unwrap();
        assert_eq!(s.consumer_seq("c1"), 1);
        let m2 = s.peek("c1").unwrap().unwrap();
        assert_eq!(m2.data, b"later");
        s.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// F70: slash names and prefix stream names must not share keys.
    #[test]
    fn stream_names_with_slash_are_isolated() {
        let dir = temp();
        {
            let mut a = Stream::open(&dir, "a").unwrap();
            a.publish(b"only-a").unwrap();
            a.close().unwrap();
        }
        {
            let mut ab = Stream::open(&dir, "a/b").unwrap();
            ab.publish(b"child").unwrap();
            assert_eq!(ab.last_seq(), 1);
            assert_eq!(ab.get(1).unwrap().data, b"child");
            ab.close().unwrap();
        }
        {
            let a = Stream::open(&dir, "a").unwrap();
            assert_eq!(a.last_seq(), 1, "stream a must not see a/b publishes");
            assert_eq!(a.get(1).unwrap().data, b"only-a");
            a.close().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

}
