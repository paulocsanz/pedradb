//! RFC-0046 P2.8 acceptance telemetry — remote read cache A/B.
//!
//! The case P2.7 left paying full price: a DECISIVE below-watermark read
//! of a key whose deciding record lives only in the remote mirror. The
//! key is in every covering segment's key set (bloom-positive — the
//! sidecar cannot and must not prune it), so pre-P2.8 every such read
//! fetched the whole object and CRC-walked it. Each round rewrites the
//! whole keyspace; a small local cap drops the older uploaded segments,
//! so the kept round's versions serve from remote objects only.
//!
//! Legs, interleaved 3x to cancel box drift:
//!   cached  — P2.8 read cache at the 64 MiB default (warm after the
//!             first read of the phase)
//!   refetch — cache disabled (0): every read fetches and walks
//!             (the pre-P2.8 cost)
//! Both legs must answer round-0's value (same verdict — correctness
//! cross-check of the cache).
//!
//! Load-sensitive by design (wall-clock ratio, not qps): absolute
//! numbers want a quiet box; the ratio is the claim.

use pedradb_core::{Db, HistoryHorizon, HistoryOptions, OpenOptions, Snapshot};
use std::fs;
use std::thread::sleep;
use std::time::{Duration, Instant};

const KEYS: usize = 8192; // k00000..k08191, fixed width so order is byte order
const ROUNDS: usize = 17; // round 0 + 16 archiving rounds/passes
const PAUSE: Duration = Duration::from_secs(3); // > window, ages the round
const READS: usize = 40;
const PHASES: usize = 3;
const CAP: u64 = 2 * 1024 * 1024; // keeps ~2-3 newest segments locally; the
                                  // older uploaded segments drop to remote-only

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

fn phase(db: &Db, snap: pedradb_core::Snapshot, want: &[u8]) -> (Duration, usize) {
    let mut hits = 0usize;
    let t = Instant::now();
    for _ in 0..READS {
        if db.get_at(snap, &key(0)).expect("get_at").as_deref() == Some(want) {
            hits += 1;
        }
    }
    (t.elapsed(), hits)
}

fn main() {
    let dir = std::env::temp_dir().join(format!("pedra-46p28-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let remote_root = dir.join("remote");
    let mut opts = OpenOptions::default();
    opts.history = HistoryOptions {
        horizon: HistoryHorizon::Window(Duration::from_secs(2)),
        cap_bytes: CAP,
        ..Default::default()
    };
    let mut db = Db::open_with(&dir, opts).expect("open");
    db.set_remote_history(pedradb_core::env::StdEnv, remote_root.clone());
    let mut rng = xorshift(0x50_32_38_5F_43_41_43_48);
    let mut val = vec![0u8; 64];

    // Detached read point (no pin — a live pin would hold the cap): the
    // ROUNDS-3 round's segment is dropped by the final round's cap while
    // still listed by the final pre-drop manifest, so the deciding record
    // is remote-only.
    let keep = ROUNDS - 3;
    let mut read_seq = None;
    let mut want = None;
    for round in 0..ROUNDS {
        for k in 0..KEYS {
            for chunk in val.chunks_mut(8) {
                chunk.copy_from_slice(&rng().to_le_bytes());
            }
            db.put(key(k).as_slice(), &val).expect("put");
            if round == keep && k == 0 {
                want = Some(val.clone()); // k00000's kept-round value
            }
        }
        db.flush().expect("flush");
        if round == keep {
            read_seq = Some(db.last_sequence());
        }
        sleep(PAUSE); // age the round past the 2 s window...
        db.compact_horizon().expect("compact_horizon"); // ...and archive it
    }
    let snap = Snapshot::at(read_seq.expect("read seq"));
    let want = want.expect("kept-round value");

    let stats = db.history_stats().expect("history_stats");
    let (remote_hists, remote_bytes) = dir_file_bytes(&remote_root, "hist");
    println!(
        "p28_ab: segments_local={} segments_remote={} remote={}B snap.seq={} \
         earliest_readable={}",
        stats.local_segments,
        remote_hists,
        remote_bytes,
        snap.sequence(),
        stats.earliest_readable,
    );
    assert!(
        snap.sequence() < stats.earliest_readable,
        "snapshot is not below the watermark; the tier leg would not run"
    );
    assert!(
        remote_hists > stats.local_segments,
        "the cap must have dropped uploaded segments (remote-only reads required)"
    );

    let mut cached_best = f64::MAX;
    let mut refetch_best = f64::MAX;
    for ph in 0..PHASES {
        // A: cache at the default budget — one warm read populates it.
        db.set_remote_read_cache(64 << 20);
        assert_eq!(
            db.get_at(snap, &key(0)).expect("warm read").as_deref(),
            Some(want.as_slice()),
            "warm read must answer the kept round's value"
        );
        let cache_stats = db.history_stats().expect("history_stats");
        assert!(
            cache_stats.remote_cache_entries > 0,
            "the warm read must populate the cache"
        );
        let (dt, hits) = phase(&db, snap, want.as_slice());
        assert_eq!(hits, READS, "cached leg must answer the kept round's value");
        let a_us = dt.as_secs_f64() * 1e6 / READS as f64;
        cached_best = cached_best.min(a_us);

        // B: cache disabled — every read refetches and walks (pre-P2.8).
        db.set_remote_read_cache(0);
        let (dt, hits) = phase(&db, snap, want.as_slice());
        assert_eq!(hits, READS, "refetch leg must answer the same value");
        let b_us = dt.as_secs_f64() * 1e6 / READS as f64;
        refetch_best = refetch_best.min(b_us);

        println!(
            "p28_ab: phase {ph}: cached {a_us:.1} us/read refetch {b_us:.1} us/read \
             cache_entries={} cache_bytes={}",
            cache_stats.remote_cache_entries, cache_stats.remote_cache_bytes,
        );
    }

    println!(
        "p28_ab: BEST cached {cached_best:.1} us/read refetch {refetch_best:.1} us/read \
         ratio {:.1}x",
        refetch_best / cached_best,
    );
    db.close().expect("close");
    let _ = fs::remove_dir_all(&dir);
}
