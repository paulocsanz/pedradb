//! RFC-0046 P2.6 acceptance telemetry — bloom sidecar read A/B.
//!
//! The case P2.5 cannot prune and P2.6 can: a NEVER-WRITTEN key whose
//! position falls inside every segment's key-coverage bound. Each round
//! writes the whole keyspace minus a fixed 16-key hole; one
//! `compact_horizon()` pass per round seals ~one segment per round, so
//! every segment's coverage spans [k00000, k08191] (bound prune keeps
//! them all) while the hole key is in none of their key sets (bloom
//! negative everywhere). A below-watermark point read of the hole key
//! must then prove `None` from the seq spans — either after CRC-walking
//! every candidate segment (~MBs each) or after reading every sidecar
//! (~KBs each).
//!
//! Legs, interleaved 3x to cancel box drift:
//!   sidecar — as sealed (bloom + range-delete intervals present)
//!   walk    — sidecars removed; a missing sidecar fails open, which is
//!             exactly the pre-P2.6 behavior
//! Both legs must answer `None` (same verdict — this is also a
//! correctness cross-check of the fail-open path).
//!
//! Load-sensitive by design (wall-clock ratio, not qps): absolute
//! numbers want a quiet box; the ratio is the claim.

use pedradb_core::{Db, HistoryHorizon, HistoryOptions, OpenOptions};
use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

const KEYS: usize = 8192; // k00000..k08191, fixed width so order is byte order
const HOLE_LO: usize = 4089; // every round skips k04089..k04104 — TARGET is
const HOLE_HI: usize = 4104; // interior to every coverage bound yet never
const TARGET: usize = 4096; // written (the P2.6-prunable key)
const ROUNDS: usize = 17; // round 0 + 16 archiving rounds/passes
const PAUSE: Duration = Duration::from_secs(3); // > window, ages the round
const READS: usize = 40;
const PHASES: usize = 3;

fn key(i: usize) -> Vec<u8> {
    format!("k{i:05}").into_bytes()
}

fn xorshift(mut x: u64) -> impl FnMut() -> u64 {
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

fn dir_file_bytes(dir: &std::path::Path, ext: &str) -> (usize, u64) {
    let mut n = 0usize;
    let mut b = 0u64;
    let Ok(rd) = fs::read_dir(dir) else {
        return (0, 0);
    };
    for e in rd.flatten() {
        if e.path().extension().is_some_and(|x| x == ext) {
            n += 1;
            b += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (n, b)
}

fn phase(db: &Db, snap: pedradb_core::Snapshot, reads: usize) -> (Duration, usize) {
    let mut nones = 0usize;
    let t = Instant::now();
    for _ in 0..reads {
        if db.get_at(snap, &key(TARGET)).expect("get_at").is_none() {
            nones += 1;
        }
    }
    (t.elapsed(), nones)
}

fn main() {
    let dir = std::env::temp_dir().join(format!("pedra-46p26-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut opts = OpenOptions::default();
    opts.history = HistoryOptions {
        horizon: HistoryHorizon::Window(Duration::from_secs(2)),
        cap_bytes: 256 * 1024 * 1024, // never hit: nothing may drop
        ..Default::default()
    };
    let mut db = Db::open_with(&dir, opts).expect("open");
    let mut rng = xorshift(0x50_32_36_42_4C_4F_4F_4D);
    let mut val = vec![0u8; 64];

    let mut snap = None;
    for round in 0..ROUNDS {
        for k in 0..KEYS {
            if (HOLE_LO..=HOLE_HI).contains(&k) {
                continue; // the hole is never written
            }
            for chunk in val.chunks_mut(8) {
                chunk.copy_from_slice(&rng().to_le_bytes());
            }
            db.put(key(k).as_slice(), &val).expect("put");
        }
        db.flush().expect("flush");
        if round == 0 {
            // Below-watermark read point: after round 0, before any
            // archiving. TARGET was never written at or below it.
            snap = Some(db.snapshot());
        }
        sleep(PAUSE); // age the round past the 2 s window...
        db.compact_horizon().expect("compact_horizon"); // ...and archive it
    }
    let snap = snap.expect("snapshot");

    let stats = db.history_stats().expect("history_stats");
    let hist = dir.join("history");
    let (segs_bloom, bloom_bytes) = dir_file_bytes(&hist, "bloom");
    println!(
        "p26_ab: segments={} archive={}B sidecars={} ({:.1}% of archive) \
         snap.seq={} earliest_readable={}",
        stats.local_segments,
        stats.local_bytes,
        segs_bloom,
        100.0 * bloom_bytes as f64 / stats.local_bytes as f64,
        snap.sequence(),
        stats.earliest_readable,
    );
    assert!(
        stats.local_segments >= ROUNDS - 1,
        "workload did not produce ~one segment per round — adjust constants"
    );
    assert!(
        snap.sequence() < stats.earliest_readable,
        "snapshot is not below the watermark; the tier leg would not run"
    );
    assert_eq!(
        segs_bloom, stats.local_segments,
        "every segment must have a sidecar"
    );

    // Stash the sidecars so legs can toggle fail-open (removed = pre-P2.6).
    let bak = dir
        .parent()
        .unwrap()
        .join(format!("pedra-46p26-bak-{}", std::process::id()));
    fs::create_dir_all(&bak).expect("bak dir");
    let sidecars: Vec<PathBuf> = fs::read_dir(&hist)
        .expect("history dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bloom"))
        .collect();
    for p in &sidecars {
        fs::copy(p, bak.join(p.file_name().unwrap())).expect("stash sidecar");
    }

    let mut a_best = f64::MAX;
    let mut b_best = f64::MAX;
    for ph in 0..PHASES {
        // A: sidecars in place (prune)
        for p in &sidecars {
            fs::copy(bak.join(p.file_name().unwrap()), p).expect("restore sidecar");
        }
        let (dt, nones) = phase(&db, snap, READS);
        assert_eq!(nones, READS, "sidecar leg must answer None");
        let a_us = dt.as_secs_f64() * 1e6 / READS as f64;
        a_best = a_best.min(a_us);

        // B: sidecars removed (fail-open walk = pre-P2.6)
        for p in &sidecars {
            fs::remove_file(p).expect("remove sidecar");
        }
        let (dt, nones) = phase(&db, snap, READS);
        assert_eq!(nones, READS, "walk leg must answer the same None");
        let b_us = dt.as_secs_f64() * 1e6 / READS as f64;
        b_best = b_best.min(b_us);

        println!(
            "p26_ab: phase {ph}: sidecar={a_us:9.1} us/read  walk={b_us:9.1} us/read  ratio={:.1}x",
            b_us / a_us,
        );
    }

    println!(
        "p26_ab: BEST sidecar={a_best:.1} us/read  walk={b_best:.1} us/read  ratio={:.1}x  \
         (bytes/read: walk≈{:.1} MiB vs sidecar≈{:.0} KiB)",
        b_best / a_best,
        stats.local_bytes as f64 / (1024.0 * 1024.0),
        bloom_bytes as f64 / 1024.0,
    );

    // Leave the tree as sealed (sidecars back) before cleanup.
    for p in &sidecars {
        fs::copy(bak.join(p.file_name().unwrap()), p).expect("restore sidecar");
    }
    drop(db);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&bak);
}
