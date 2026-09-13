//! RFC-0217 P0.1 — bounded collection window for the async group leader,
//! on the real path (`ConcurrentDb`, real fs, default write sync off —
//! the parity async class).
//!
//! Pins:
//! 1. window on ⇒ two async writers merge into real groups
//!    (`avg_grp > 1.2`; the p211p/p211q clean arm pinned `avg_grp ==
//!    1.00` on every count because async-only groups drained instantly);
//! 2. the lone writer never parks behind a leader and never pays the
//!    window (single-client p50 untouched by construction);
//! 3. the AS-IS twin: window off keeps the 0201 boundary — two writers
//!    ≤ ncpu stay on the bypass, nobody ever queues behind a leader,
//!    every commit 1-op;
//! 4. grouped commits survive reopen (real path end-to-end);
//! 5. P0.1b: the leader collects through the peer's client-side gap —
//!    a slow writer (inter-op sleep ≫ the leader's cycle) is absorbed
//!    anyway, because the wait targets the gap ghost, not the queue.

use pedradb_core::concurrent::ConcurrentDb;
use pedradb_core::db::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn tmp(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("rfc0217-window-{tag}-{n}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open_async(tag: &str) -> ConcurrentDb<pedradb_sim::FailingEnvArc> {
    let db = ConcurrentDb::open_with_env(
        &tmp(tag),
        OpenOptions {
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            ..OpenOptions::default()
        },
        pedradb_sim::FailingEnvArc::passing(),
    )
    .unwrap();
    db.set_default_write_sync(false);
    db
}

fn put_rounds(db: &ConcurrentDb<pedradb_sim::FailingEnvArc>, tag: u8, rounds: usize) {
    for i in 0..rounds {
        db.put(&[b'g', tag, i as u8], b"v").unwrap();
    }
}

fn assert_visible(db: &ConcurrentDb<pedradb_sim::FailingEnvArc>, tag: u8, rounds: usize) {
    for i in 0..rounds {
        assert_eq!(
            db.get(&[b'g', tag, i as u8]).as_deref(),
            Some(b"v".as_ref()),
            "grouped commit missing: tag {tag} round {i}"
        );
    }
}

#[test]
fn rfc0217_group_window_forms_group_at_two_writers() {
    let db = open_async("on");
    db.set_group_window(Duration::from_millis(1)); // ceiling (1000 µs)

    let n = 2usize;
    let rounds = 250usize;
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::new();
    for t in 0..n {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            put_rounds(&db, t as u8, rounds);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let (_submits, queued, groups, group_ops) = db.write_group_stats();
    assert!(queued >= 1, "a writer must have parked behind an active leader");
    assert!(groups >= 1, "window must route 2 async writers through the group path");
    let avg = group_ops as f64 / groups as f64;
    assert!(
        avg > 1.2,
        "window must coalesce arrivals: avg_grp {avg:.2} (p211p clean arm pinned 1.00)"
    );
    assert_visible(&db, 0, rounds);
    assert_visible(&db, 1, rounds);
}

#[test]
fn rfc0217_group_window_lone_writer_never_parks_or_waits() {
    let db = open_async("lone");
    db.set_group_window(Duration::from_millis(1));
    let rounds = 200usize;

    let t0 = Instant::now();
    put_rounds(&db, b'l', rounds);
    let elapsed = t0.elapsed();

    let (_submits, queued, _groups, _group_ops) = db.write_group_stats();
    assert_eq!(
        queued, 0,
        "lone async writer must never park behind a leader (no leader exists)"
    );
    // A mis-wired window taxes every lone commit the full 1000 µs:
    // 200 rounds would burn ≥ 200 ms. The lone fast path stays µs-scale.
    assert!(
        elapsed < Duration::from_millis(150),
        "lone writer paid the window: {elapsed:?} for {rounds} puts"
    );
    assert_visible(&db, b'l', rounds);
}

#[test]
fn rfc0217_group_window_off_keeps_the_0201_boundary_twin() {
    if std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        < 2
    {
        eprintln!("1-cpu box: policy merges writers > ncpu by itself; twin assert skipped");
        return;
    }
    let db = open_async("twin"); // window OFF (default = AS-IS)

    let n = 2usize;
    let rounds = 250usize;
    let barrier = Arc::new(Barrier::new(n));
    let mut handles = Vec::new();
    for t in 0..n {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            put_rounds(&db, t as u8, rounds);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let (_submits, queued, groups, group_ops) = db.write_group_stats();
    assert_eq!(
        queued, 0,
        "AS-IS twin: 2 writers ≤ ncpu stay on the bypass — nobody parks behind a leader"
    );
    let avg = group_ops as f64 / groups as f64;
    assert!(
        (avg - 1.0).abs() < 0.05,
        "AS-IS twin: no coalescing without the window (avg_grp {avg:.2}, p211p pinned 1.00)"
    );
    assert_visible(&db, 0, rounds);
    assert_visible(&db, 1, rounds);
}

#[test]
fn rfc0217_group_window_collects_through_client_gap() {
    // P0.1b discriminator: writer B sleeps 150µs between puts (inside
    // the 250µs recent-concurrency horizon, far above the leader's ~µs
    // cycle), A hammers continuously. A flat-window leader that only
    // absorbs already-queued arrivals parks B just by luck of overlap;
    // a collect leader holds through the gap and absorbs B almost
    // every time.
    let db = open_async("gap");
    db.set_group_window(Duration::from_millis(1));

    let b_rounds = 80usize;
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    {
        let db = db.clone();
        let stop = Arc::clone(&stop);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let mut i = 0usize;
            while !stop.load(Ordering::Relaxed) {
                db.put(&[b'g', b'a', i as u8], b"v").unwrap();
                i = i.wrapping_add(1);
            }
        }));
    }
    {
        let db = db.clone();
        let stop = Arc::clone(&stop);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for i in 0..b_rounds {
                db.put(&[b'g', b'b', i as u8], b"v").unwrap();
                std::thread::sleep(Duration::from_micros(150));
            }
            stop.store(true, Ordering::Relaxed);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let (_submits, _queued, groups, group_ops) = db.write_group_stats();
    let absorbed = group_ops.saturating_sub(groups);
    assert!(
        absorbed as usize >= (b_rounds * 7) / 10,
        "collect must absorb the gap ghost: absorbed {absorbed} of {b_rounds} slow-writer ops \
         (groups {groups}, ops {group_ops})"
    );
    for i in 0..b_rounds {
        assert_eq!(
            db.get(&[b'g', b'b', i as u8]).as_deref(),
            Some(b"v".as_ref()),
            "slow writer op {i} missing"
        );
    }
}

#[test]
fn rfc0217_group_window_grouped_commits_survive_reopen() {
    let dir = tmp("reopen");
    let db = ConcurrentDb::open_with_env(
        &dir,
        OpenOptions {
            sync: false,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            ..OpenOptions::default()
        },
        pedradb_sim::FailingEnvArc::passing(),
    )
    .unwrap();
    db.set_default_write_sync(false);
    db.set_group_window(Duration::from_millis(1));

    let rounds = 250usize;
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for t in 0..2u8 {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            put_rounds(&db, t, rounds);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let (_s, _q, groups, _ops) = db.write_group_stats();
    assert!(groups >= 1);
    drop(db);

    let db2 = ConcurrentDb::open_with_env(
        &dir,
        OpenOptions::default(),
        pedradb_sim::FailingEnvArc::passing(),
    )
    .unwrap();
    assert_visible(&db2, 0, rounds);
    assert_visible(&db2, 1, rounds);
    let _ = std::fs::remove_dir_all(dir);
}
