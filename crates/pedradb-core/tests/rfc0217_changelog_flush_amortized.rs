//! RFC-0217 P1.1 integration — amortized explicit-flush CHANGELOG store.
//!
//! Every test drives the real shipped paths: `Db::flush` /
//! `ConcurrentDb::flush` (deferred store + WAL-archive rotate + imm-delta
//! staging), `Db::open` (archive replay, feed-only), `Db::close` (store
//! point), and the kernel debounce/cap gates. Nothing is re-implemented
//! here.
//!
//! Durability contract under test (unchanged from F212): after any crash
//! point, reopen reconstructs exactly the feed a synchronous per-flush
//! store would have left — via the archived WAL segments when the on-disk
//! cache lags.

use bytes::Bytes;
use pedradb_core::{ConcurrentDb, Db};
use std::collections::HashSet;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rfc0217-p11-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wal_archive_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("WAL.arch"))
                .collect()
        })
        .unwrap_or_default()
}

fn key(i: usize) -> Vec<u8> {
    format!("ch/{i:06}").into_bytes()
}

/// Below the debounce an explicit flush stores nothing, archives the
/// rotated WAL, and the staged imm delta keeps the in-memory feed complete.
/// A drop without `close` (crash) still reopens every key and a complete
/// feed: the deferred MANIFEST publish means the archives are the only
/// durable copy of that window, so open replays them **as data** and keeps
/// the files until a durability point (close) publishes and clears them.
#[test]
fn explicit_flush_defers_store_and_crash_reopen_is_complete() {
    let dir = scratch("defer-crash");
    {
        let mut db = Db::open(&dir).unwrap();
        db.set_changelog_interval(0);
        for i in 0..24 {
            db.put(key(i), b"v").unwrap();
        }
        db.flush().unwrap();
        assert_eq!(db.changelog_store_count(), 0, "deferred below debounce");
        assert_eq!(db.wal_archive_count(), 1, "rotate archived the segment");
        assert_eq!(
            db.changes_after(0).len(),
            24,
            "staged imm delta keeps the feed complete after rotate"
        );
        // More writes + another deferred flush: two archive slots, still no store.
        for i in 24..48 {
            db.put(key(i), b"v").unwrap();
        }
        db.flush().unwrap();
        assert_eq!(db.changelog_store_count(), 0);
        assert_eq!(db.wal_archive_count(), 2);
        assert_eq!(db.changes_after(0).len(), 48);
        std::mem::drop(db); // crash point: no close, no debounced store
    }
    assert_eq!(wal_archive_files(&dir).len(), 2, "segments survive the crash");
    let mut db = Db::open(&dir).unwrap();
    let feed: HashSet<Vec<u8>> = db.changes_after(0).into_iter().map(|e| e.key.to_vec()).collect();
    for i in 0..48 {
        assert!(feed.contains(&key(i)), "reopen feed missing {}", key(i).len());
        // Data replay: the deferred publish left the archives as the only
        // durable copy — the keys must serve, not just feed-extend.
        assert_eq!(
            db.get(&key(i)).as_deref(),
            Some(&b"v"[..]),
            "reopen data missing for op {i}"
        );
    }
    assert_eq!(
        wal_archive_files(&dir).len(),
        2,
        "uncovered segments stay: they are the only durable copy"
    );
    // The replayed window lives in the memtable — a flush moves it into
    // SSTs and the close-time publish covers the segments, clearing them.
    db.flush().unwrap();
    db.close().unwrap();
    assert!(
        wal_archive_files(&dir).is_empty(),
        "flush + close publishes the MANIFEST then clears the segments"
    );
    let db = Db::open(&dir).unwrap();
    for i in 0..48 {
        assert_eq!(db.get(&key(i)).as_deref(), Some(&b"v"[..]));
    }
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The debounce gate fires mid-run, the store covers every archived
/// segment (files cleared, watermark == last_sequence), and reopen sees
/// the full feed.
#[test]
fn explicit_flush_debounce_fires_and_clears_archives() {
    let dir = scratch("debounce");
    {
        let mut db = Db::open(&dir).unwrap();
        db.set_changelog_interval(0).set_changelog_flush_debounce(4);
        for round in 0..5usize {
            for i in 0..8 {
                db.put(key(round * 8 + i), b"v").unwrap();
            }
            db.flush().unwrap();
        }
        assert_eq!(
            db.changelog_store_count(),
            1,
            "debounce 4 stores exactly at the 4th flush"
        );
        // The 5th flush opens a new debounce window: one archived segment,
        // deferred — never a second full store.
        assert_eq!(db.wal_archive_count(), 1, "new window archives defers");
        assert_eq!(db.changes_after(0).len(), 40);
        db.close().unwrap();
    }
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.changes_after(0).len(), 40);
    assert!(wal_archive_files(&dir).is_empty());
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A full archive chain forces the synchronous store at the very next
/// rotate — the chain never grows past the cap.
#[test]
fn archive_cap_forces_synchronous_store() {
    let dir = scratch("cap");
    {
        let mut db = Db::open(&dir).unwrap();
        db.set_changelog_interval(0)
            .set_wal_archive_cap(2)
            .set_changelog_flush_debounce(1000);
        for round in 0..4usize {
            for i in 0..4 {
                db.put(key(round * 4 + i), b"v").unwrap();
            }
            db.flush().unwrap();
        }
        assert!(
            db.changelog_store_count() >= 1,
            "cap 2 over 4 flushes must force a store"
        );
        assert!(db.wal_archive_count() <= 2, "chain bounded by the cap");
        assert_eq!(db.changes_after(0).len(), 16);
        db.close().unwrap();
    }
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.changes_after(0).len(), 16);
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The kafka_changelog shape itself on the concurrent engine (compat
/// backing): batch + explicit flush per op, feed parity across a crash at
/// every point of the deferred window.
#[test]
fn concurrent_kafka_shape_feed_parity_across_crash() {
    let dir = scratch("kafka-shape");
    const OPS: usize = 40;
    const BATCH: usize = 32;
    {
        let db = ConcurrentDb::open(&dir).unwrap();
        for op in 0..OPS {
            let batch: Vec<_> = (0..BATCH)
                .map(|j| pedradb_core::BatchOp::put(key(op * BATCH + j), b"v"))
                .collect();
            db.apply_batch_vec(batch).unwrap();
            db.flush().unwrap();
        }
        let feed: HashSet<Vec<u8>> = db.changes_after(0).into_iter().map(|e| e.key.to_vec()).collect();
        assert_eq!(feed.len(), OPS * BATCH, "live feed complete");
        std::mem::drop(db); // crash point
    }
    let db = Db::open(&dir).unwrap();
    let feed: HashSet<Vec<u8>> = db.changes_after(0).into_iter().map(|e| e.key.to_vec()).collect();
    assert_eq!(
        feed.len(),
        OPS * BATCH,
        "reopen after crash must rebuild the full feed from archives/cache"
    );
    // Data parity too: the deferred publish leaves archives as the only
    // durable copy — every key must serve after the crash.
    for op in 0..OPS * BATCH {
        assert_eq!(
            db.get(&key(op)).as_deref(),
            Some(&b"v"[..]),
            "reopen data missing for op {op}"
        );
    }
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A WAL-frame put whose sequence sits BELOW a published bulk-run floor
/// (bulk runs interleave their sequences with normal commits) must never
/// ride an archived segment: replay drops frames ≤ the floor, so that
/// rotate publishes instead of archiving. Crash right after the flush —
/// both the plain put and the latched bulk keys have to serve.
#[test]
fn below_floor_window_publishes_instead_of_archiving() {
    let dir = scratch("below-floor");
    {
        let db = ConcurrentDb::open_with(
            &dir,
            pedradb_core::OpenOptions {
                sync: false,
                ..pedradb_core::OpenOptions::default()
            },
        )
        .unwrap();
        // Plain WAL puts first (low sequences)...
        for i in 0..4 {
            db.put(format!("plain/{i}").into_bytes(), b"v").unwrap();
        }
        // ...then a latched bulk batch whose run sequences land above them.
        let mut keys = Vec::new();
        for j in 0..32usize {
            keys.push(Bytes::from(format!("bulk/{j:04}").into_bytes()));
        }
        let vals = vec![Bytes::from_static(b"b"); keys.len()];
        db.set_physical_cfs(vec!["bulk".into()]);
        for round in 0..12u32 {
            let ks: Vec<_> = keys
                .iter()
                .map(|k| Bytes::from(format!("{}/{round}", String::from_utf8_lossy(k)).into_bytes()))
                .collect();
            db.apply_latched_bulk("bulk", ks, vals.clone(), Vec::new())
                .unwrap();
        }
        db.flush().unwrap();
        std::mem::drop(db); // crash before any gate
    }
    let db = Db::open(&dir).unwrap();
    for i in 0..4 {
        assert_eq!(
            db.get(format!("plain/{i}").as_bytes()).as_deref(),
            Some(&b"v"[..]),
            "below-floor plain put lost: {i}"
        );
    }
    for j in 0..32usize {
        let k = format!("bulk/{j:04}/11");
        assert_eq!(
            db.get(k.as_bytes()).as_deref(),
            Some(&b"b"[..]),
            "bulk key lost: {k}"
        );
    }
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The deferred publish itself: below the debounce the MANIFEST on disk
/// must not advance (the per-flush SST fdatasync + rewrite is the cost
/// this slice removes); the crash reopen sweeps the orphan SST and serves
/// every key from the archive replay.
#[test]
fn deferred_rotate_skips_manifest_publish_until_gate() {
    let dir = scratch("defer-publish");
    // Each publish installs a new MANIFEST-000N and repoints CURRENT:
    // the pointer's generation is the publish counter.
    let manifest_gen = |dir: &std::path::Path| -> u64 {
        std::fs::read_to_string(dir.join("CURRENT"))
            .unwrap_or_default()
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix("MANIFEST-"))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    {
        let mut db = Db::open(&dir).unwrap();
        db.set_changelog_interval(0);
        let first_open = manifest_gen(&dir);
        for i in 0..8 {
            db.put(key(i), b"v").unwrap();
        }
        db.flush().unwrap();
        assert_eq!(db.changelog_store_count(), 0, "below the debounce");
        assert_eq!(
            manifest_gen(&dir),
            first_open,
            "deferred rotate must not publish the MANIFEST per flush"
        );
        assert_eq!(db.wal_archive_count(), 1);
        std::mem::drop(db); // crash before any gate
    }
    let mut db = Db::open(&dir).unwrap();
    for i in 0..8 {
        assert_eq!(db.get(&key(i)).as_deref(), Some(&b"v"[..]));
    }
    // The reopen holds the window in the memtable (replayed from the
    // archives); a flush + publish lands it in a listed SST.
    db.flush().unwrap();
    db.close().unwrap();
    assert!(
        manifest_gen(&dir) > 0,
        "flush + close publishes the inventory"
    );
    assert!(wal_archive_files(&dir).is_empty(), "covered segments clear");
    let db = Db::open(&dir).unwrap();
    for i in 0..8 {
        assert_eq!(db.get(&key(i)).as_deref(), Some(&b"v"[..]));
    }
    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
