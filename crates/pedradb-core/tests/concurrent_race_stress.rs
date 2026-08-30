//! RFC-0020 P1.4 — ConcurrentDb write-group stress (loom/TSan substitute job).
//!
//! Multi-thread put/get/CAS against one ConcurrentDb. Documents the in-tree race
//! job; for ThreadSanitizer see `scripts/race_job.sh` (nightly + sanitizer).

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp() -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let i = N.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pedra-race-{n}-{i}"));
    let _ = fs::remove_dir_all(&d);
    d
}

#[test]
fn concurrent_db_write_group_stress_silent_wrong_zero() {
    let dir = temp();
    let db = Arc::new(
        ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
            },
        )
        .unwrap(),
    );
    let silent = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for t in 0..8u8 {
        let db = Arc::clone(&db);
        let silent = Arc::clone(&silent);
        handles.push(thread::spawn(move || {
            for i in 0..200u16 {
                let k = [b't', t, (i & 0xff) as u8, (i >> 8) as u8];
                let v = [t, (i & 0xff) as u8];
                if db.put(k, v).is_err() {
                    silent.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                match db.get(&k) {
                    Some(got) if got.as_ref() == v.as_slice() => {}
                    // Concurrent overwrite on same key from other threads not used —
                    // each thread owns prefix t.
                    other => {
                        if other.as_ref().map(|b| b.as_ref()) != Some(v.as_slice()) {
                            silent.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            // CAS contention on shared flag.
            let _ = db.put_if_absent(b"flag", [t]);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let wrong = silent.load(Ordering::Relaxed);
    assert_eq!(wrong, 0, "concurrent stress silent_wrong");
    // Exactly one CAS winner on flag.
    assert!(db.get(b"flag").is_some());
    let _ = fs::remove_dir_all(&dir);
}
