//! Lucas' async-publish race, stressed: seq is assigned before the WAL
//! lock and `publish_sequence` is a CAS-max, so a later sequence can
//! become visible before an earlier one is inserted into the memtable.
//! The invariant that must survive regardless: once `put(k, v)` returns
//! Ok (its WAL frame, memtable insert, publish and ack all completed),
//! every subsequent `get(k)` observes a value at or after `v` — the
//! caches must not serve a stale negative captured during the
//! out-of-order window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::{BatchOp, OpenOptions};

#[test]
fn async_bypass_read_your_writes_under_out_of_order_publish() {
    let dir = std::env::temp_dir().join(format!("lin-{0}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut oo = OpenOptions::default();
    oo.auto_flush_bytes = Some(4 << 20);
    let db = Arc::new(ConcurrentDb::open_with(dir.clone(), oo).expect("open"));

    const WRITERS: usize = 3;
    const OPS: u64 = 20_000;
    let stop = Arc::new(AtomicBool::new(false));
    let violations = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();

    // Writers: 1-op async puts (the bypass path Lucas audited), each on
    // its own key so a stale answer for one writer is unambiguous.
    for w in 0..WRITERS {
        let db = Arc::clone(&db);
        let stop = Arc::clone(&stop);
        let violations = Arc::clone(&violations);
        handles.push(std::thread::spawn(move || {
            let key = format!("lin.writer-{w:02}").into_bytes();
            for i in 1..=OPS {
                let val = format!("v{w:02}.{i:08}").into_bytes();
                let op = BatchOp::put(&key, &val);
                // 1-op apply = the async bypass shape.
                db.apply_batch_vec(vec![op]).expect("apply");
                // Read-your-writes after ack, on a FRESH get each time.
                let got = db.get(&key);
                let expect_tail = format!(".{i:08}");
                match got {
                    Some(v) => {
                        let s = String::from_utf8_lossy(&v);
                        if !s.ends_with(&expect_tail) {
                            violations.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        violations.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
        }));
    }

    // A cross-reader: samples writer 0's key continuously and asserts
    // monotonic visibility (never goes BACKWARD — that would be a stale
    // cached answer from the out-of-order window).
    let db_r = Arc::clone(&db);
    let stop_r = Arc::clone(&stop);
    let violations_r = Arc::clone(&violations);
    handles.push(std::thread::spawn(move || {
        let key = b"lin.writer-00".to_vec();
        let mut highest: u64 = 0;
        while !stop_r.load(Ordering::Relaxed) {
            if let Some(v) = db_r.get(&key) {
                let s = String::from_utf8_lossy(&v);
                if let Some(idx) = s.rfind('.') {
                    if let Ok(n) = s[idx + 1..].parse::<u64>() {
                        if n < highest {
                            // Went backward: a stale answer escaped the
                            // invalidation — the publish-order hazard.
                            violations_r.fetch_add(1, Ordering::Relaxed);
                        }
                        highest = highest.max(n);
                    }
                }
            }
        }
    }));

    for h in handles.drain(..WRITERS) {
        h.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().unwrap();
    }

    let v = violations.load(Ordering::Relaxed);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        v, 0,
        "read-your-writes/monotonic-visibility violations under concurrent \
         async-publish: {v} (out-of-order publish served stale answers)"
    );
}
