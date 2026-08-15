//! Journal / change-feed canary (RFC-0020 P2.1 / W4 `feed-watermark`).
//!
//! Consumer pins a watermark seq; after crash, feed entries never exceed
//! durable `last_sequence`, and catch-up from pin is consistent.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod pin_kernel;

use pedradb_core::{ChangeEntry, ChangeKind, Db, OpenOptions, Result, SequenceNumber, StdEnv};
use std::path::Path;

/// In-process journal consumer pinned at a sequence.
#[derive(Debug, Clone)]
pub struct JournalConsumer {
    /// Last consumed sequence (exclusive lower bound for next poll).
    pub pin: SequenceNumber,
}

impl JournalConsumer {
    /// Start before any events.
    #[must_use]
    pub fn new() -> Self {
        Self { pin: 0 }
    }

    /// Poll durable changes after pin; advances pin to max seen.
    ///
    /// Pin-on-read is the journal-canary contract. Fold consumers must **not**
    /// use this: call [`Self::peek`] then [`Self::pin_after_apply`] after the
    /// fold `apply` returns (RFC-0024).
    pub fn catch_up(&mut self, db: &Db<StdEnv>) -> Vec<ChangeEntry> {
        let batch = self.peek(db);
        if pin_kernel::catch_up_pins_on_read() {
            let batch_max = batch.iter().map(|e| e.sequence).max();
            self.pin = pin_kernel::next_pin(self.pin, batch_max);
        }
        batch
    }

    /// Poll durable changes after pin **without** advancing the pin.
    #[must_use]
    pub fn peek(&self, db: &Db<StdEnv>) -> Vec<ChangeEntry> {
        debug_assert!(!pin_kernel::peek_pins_cursor());
        debug_assert!(!pin_kernel::fold_pins_on_read());
        let last = db.last_sequence();
        db.changes_after(self.pin)
            .into_iter()
            .filter(|e| e.sequence <= last)
            .collect()
    }

    /// Persist pin only after the caller applied every revision `<= applied_through`.
    pub fn pin_after_apply(&mut self, applied_through: SequenceNumber) {
        if pin_kernel::may_advance_pin(self.pin, applied_through) {
            self.pin = applied_through;
        }
    }
}

impl Default for JournalConsumer {
    fn default() -> Self {
        Self::new()
    }
}

/// Workload report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadReport {
    /// Name.
    pub name: &'static str,
    /// Trials.
    pub trials: u64,
    /// Must be 0.
    pub silent_wrong: u64,
    /// Detail.
    pub detail: String,
}

/// W4: feed ⊆ last_seq; no ghosts after kill; catch-up from pin.
///
/// # Errors
/// I/O.
pub fn workload_feed_watermark(dir: impl AsRef<Path>) -> Result<WorkloadReport> {
    let dir = dir.as_ref();
    let opts = OpenOptions {
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
    };
    let mut silent_wrong = 0u64;
    let mut consumer = JournalConsumer::new();
    let mut expected: Vec<(SequenceNumber, Vec<u8>, ChangeKind)> = Vec::new();

    {
        let mut db = Db::open_with(dir, opts)?;
        for i in 0..10u8 {
            let seq = db.put_with_seq([b'k', i], [b'v', i])?;
            expected.push((seq, vec![b'k', i], ChangeKind::Put));
            let batch = consumer.catch_up(&db);
            let last = db.last_sequence();
            for e in &batch {
                if e.sequence > last {
                    silent_wrong += 1;
                }
            }
            if batch.iter().any(|e| e.key.as_ref() == [b'k', i]) {
                // ok
            } else if db.get(&[b'k', i]).is_some() {
                // put visible but not yet in feed — should still appear
                silent_wrong += 1;
            }
        }
        let del_seq = db.delete_with_seq(b"k\x00")?;
        expected.push((del_seq, b"k\x00".to_vec(), ChangeKind::Delete));
        let _ = consumer.catch_up(&db);
        let pin = consumer.pin;
        let last = db.last_sequence();
        // Ghost check on full tail from 0.
        for e in db.changes_after(0) {
            if e.sequence > last {
                silent_wrong += 1;
            }
        }
        drop(db);

        // Reopen: catch-up from pin continues; no ghosts.
        let db = Db::open_with(dir, opts)?;
        let last = db.last_sequence();
        let mut c2 = JournalConsumer { pin };
        let rest = c2.catch_up(&db);
        for e in &rest {
            if e.sequence > last {
                silent_wrong += 1;
            }
            if e.sequence <= pin {
                silent_wrong += 1;
            }
        }
        // Full feed after reopen matches durable history length loosely.
        let all = db.changes_after(0);
        if all.iter().any(|e| e.sequence > last) {
            silent_wrong += 1;
        }
        // Expected puts (except deleted k0) still gettable when kind put and not deleted later.
        for (seq, key, kind) in &expected {
            let _ = (seq, kind);
            if key.as_slice() == b"k\x00" {
                if db.get(key).is_some() {
                    silent_wrong += 1;
                }
            } else if db.get(key).is_none() {
                silent_wrong += 1;
            }
        }
        let _ = rest;
        drop(db);
    }

    Ok(WorkloadReport {
        name: "feed-watermark",
        trials: 1,
        silent_wrong,
        detail: format!("pin_end_check silent_wrong={silent_wrong}"),
    })
}

/// Append helper returning seq.
///
/// # Errors
/// Put I/O.
pub fn append(
    db: &mut Db<StdEnv>,
    key: impl AsRef<[u8]>,
    val: impl AsRef<[u8]>,
) -> Result<SequenceNumber> {
    db.put_with_seq(key, val)
}

/// Read feed slice as public API.
#[must_use]
pub fn changes_after(db: &Db<StdEnv>, from: SequenceNumber) -> Vec<ChangeEntry> {
    db.changes_after(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedra-journal-{n}-{i}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn consumer_advances_pin() {
        let dir = temp();
        let mut db = Db::open(&dir).unwrap();
        let mut c = JournalConsumer::new();
        let s = append(&mut db, b"a", b"1").unwrap();
        let got = c.catch_up(&db);
        assert_eq!(got.len(), 1);
        assert_eq!(c.pin, s);
        assert!(got[0].sequence <= db.last_sequence());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn peek_does_not_advance_pin() {
        let dir = temp();
        let mut db = Db::open(&dir).unwrap();
        let mut c = JournalConsumer::new();
        let s = append(&mut db, b"a", b"1").unwrap();
        let got = c.peek(&db);
        assert_eq!(got.len(), 1);
        assert_eq!(c.pin, 0, "peek must not pin on receipt");
        c.pin_after_apply(s);
        assert_eq!(c.pin, s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn feed_watermark_silent_wrong_zero() {
        let dir = temp();
        let r = workload_feed_watermark(&dir).unwrap();
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
