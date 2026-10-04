//! RFC-0331 V5.1: WAL ≥ 1 GiB recovery under a physically torn tail.
//!
//! Writes ~1 GiB of real WAL on the real filesystem (async commits, the
//! production class), truncates the tail INTO the last record (a torn
//! tail, as a power loss mid-frame would leave), and reopens. Acceptance:
//! the reopen either serves the recovered prefix (torn tails are routine:
//! every key before the tear horizon is present) or fails closed with a
//! NAMED recovery error — never silent-wrong data.

#![forbid(unsafe_code)]

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::time::Instant;

const VAL: usize = 16 * 1024;
const BATCH: usize = 64;
/// Keys well inside the written prefix, sampled after reopen.
const SAMPLE_STRIDE: usize = 977;

fn temp_db_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pedra-rfc0331-{prefix}-{nanos}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn rfc0331_v51_gib_wal_torn_tail_reopen_serves_prefix() {
    let dir = temp_db_dir("gib-tear");
    let wal_path = dir.join(pedradb_core::db::WAL_FILE_NAME);
    let val = vec![b'G'; VAL];
    let mut written = 0usize;

    // Phase 1: grow the WAL past 1 GiB with async commits (no per-op fsync —
    // the production write class; the durability comes from close()).
    {
        let db = ConcurrentDb::open_with(
            &dir,
            OpenOptions {
                sync: false,
                exclusive: true,
                // Keep every byte in memtable+WAL: the default 4 MiB
                // auto-flush moves data to SSTs and PRUNES the WAL, so it
                // plateaus at ~3 MiB and a 1 GiB WAL never materializes.
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                ..Default::default()
            },
        )
        .expect("open");
        let t0 = Instant::now();
        // Hard ceiling: the loop can never spin forever if growth stalls.
        let mut guard = 0usize;
        while wal_path.metadata().map(|m| m.len()).unwrap_or(0) < (1 << 30) {
            assert!(guard < 400_000, "WAL growth stalled below 1 GiB");
            guard += BATCH;
            for _ in 0..BATCH {
                db.put(format!("k/{written:08}").as_bytes(), &val)
                    .expect("async put");
                written += 1;
            }
        }
        eprintln!(
            "V5.1: wrote {written} keys ({} MiB WAL) in {:?}",
            wal_path.metadata().unwrap().len() / (1 << 20),
            t0.elapsed()
        );
        // Crash semantics: leak the handle — no close(), no flush, no WAL
        // pruning. The torn-tail reopen below must recover from the WAL
        // exactly as a power loss would find it.
        std::mem::forget(db);
    }

    // Phase 2: physically tear the tail mid-record (a power loss in the
    // middle of the final frame leaves exactly this shape).
    let full = wal_path.metadata().unwrap().len();
    let torn = full - 41;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&wal_path)
        .unwrap();
    f.set_len(torn).expect("tear");
    drop(f);

    // Phase 3: reopen — prefix served (torn tails are routine) or a NAMED
    // fail-closed error. Anything else is silent-wrong.
    match ConcurrentDb::open(&dir) {
        Ok(db) => {
            let mut sampled = 0;
            let horizon = written - 10 * BATCH; // well inside the prefix
            for i in (0..horizon).step_by(SAMPLE_STRIDE) {
                let v = db
                    .get(format!("k/{i:08}").as_bytes())
                    .unwrap_or_else(|| panic!("prefix key {i} lost after torn-tail reopen"));
                assert_eq!(v.as_ref(), val.as_slice(), "key {i} corrupted");
                sampled += 1;
            }
            eprintln!("V5.1: prefix served, {sampled} sampled keys intact");
            db.close().unwrap();
        }
        Err(e) => {
            let msg = format!("{e:?}");
            assert!(
                msg.contains("resync")
                    || msg.contains("Truncated")
                    || msg.contains("corrupt")
                    || msg.contains("CRC")
                    || msg.contains("Recovery"),
                "unnamed reopen failure after torn tail: {msg}"
            );
            eprintln!("V5.1: fail-closed with a NAMED error: {msg}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
