//! RFC-0046 acceptance telemetry — disk sizing A/B on an overwrite workload.
//!
//! Same schedule, two retention profiles:
//!   all    — `HistoryHorizon::All`: the kernel default before 2026-08-21
//!            (keep every version forever; nothing is ever GCed).
//!   window — `Window(2 s)` + 4 MiB archive cap: the RFC-0046 mechanism at
//!            bench-friendly constants (the product default is `Window(24 h)`
//!            + 1 GiB cap — same bound, bigger numbers).
//!
//! Workload: 256 keys × 1 KiB incompressible payload, 40 full-overwrite
//! rounds, explicit flush per round, 3 s aging pause every 4 rounds so
//! versions cross the short horizon mid-run. Measures db bytes (dir minus
//! `history/`), archive bytes, total, written and live set.
//!
//! Load-insensitive by design (byte accounting, not qps).

use pedradb_core::{Db, HistoryHorizon, HistoryOptions, OpenOptions};
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

const KEYS: usize = 256;
const ROUNDS: usize = 40;
const PAUSE_EVERY: usize = 4;
const PAUSE: Duration = Duration::from_secs(3);

fn xorshift(mut x: u64) -> impl FnMut() -> u64 {
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

fn dir_bytes_split(root: &Path) -> (u64, u64) {
    // (db bytes, history bytes) — everything under `history/` is the archive.
    let mut db = 0u64;
    let mut hist = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if path.components().any(|c| c.as_os_str() == "history") {
                    hist += len;
                } else {
                    db += len;
                }
            }
        }
    }
    (db, hist)
}

fn run(label: &str, history: HistoryOptions) {
    let mut opts = OpenOptions::default();
    opts.history = history;
    run_opts(label, opts);
}

fn run_opts(label: &str, mut opts: OpenOptions) {
    if label == "trigger" {
        opts.auto_compact_sst_count = Some(8);
    }
    let dir = std::env::temp_dir().join(format!("pedra-46sizing-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut db = Db::open_with(&dir, opts).expect("open");
    let mut rng = xorshift(0x4643_5349_5A49_4E47);
    let mut key = [0u8; 8];
    let mut val = vec![0u8; 1024];

    let written: u64 = (KEYS * ROUNDS) as u64 * 1024;
    for round in 0..ROUNDS {
        for k in 0..KEYS {
            key.copy_from_slice(&rng().to_le_bytes());
            for chunk in val.chunks_mut(8) {
                chunk.copy_from_slice(&rng().to_le_bytes());
            }
            let mut kbuf = Vec::with_capacity(16);
            kbuf.extend_from_slice(format!("k{k:04}").as_bytes());
            kbuf.extend_from_slice(&key[..2]); // spread, still same key set
            db.put(kbuf.as_slice(), &val).expect("put");
        }
        db.flush().expect("flush");
        if std::env::var("SIZING_DEBUG").is_ok() {
            eprintln!(
                "round {round:2}: earliest={} ssts={} sst_bytes={}",
                db.earliest_readable_sequence(),
                db.stats().sst_count,
                db.stats().sst_bytes,
            );
        }
        if (round + 1) % PAUSE_EVERY == 0 {
            sleep(PAUSE); // let versions cross the (short) horizon
        }
    }
    // Final aging + flush so the window profile GCs what it can.
    sleep(PAUSE);
    db.flush().expect("final flush");
    db.close().expect("close");

    let (db_bytes, hist_bytes) = dir_bytes_split(&dir);
    let live: u64 = (KEYS) as u64 * 1024;
    println!(
        "sizing_ab: {label:<6} live_set={live}B written={written}B \
         db={db_bytes}B history={hist_bytes}B total={}B \
         total_over_live={:.1}x total_over_written={:.2}x",
        db_bytes + hist_bytes,
        (db_bytes + hist_bytes) as f64 / live as f64,
        (db_bytes + hist_bytes) as f64 / written as f64,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    // SIZING_PROFILE=all|window runs one profile (default: both).
    let only = std::env::var("SIZING_PROFILE").unwrap_or_default();
    println!("sizing_ab: keys={KEYS} x 1KiB, {ROUNDS} overwrite rounds, flush/round, pause {PAUSE:?} x{}", ROUNDS / PAUSE_EVERY);
    if only.is_empty() || only == "all" {
        run(
            "all",
            HistoryOptions { horizon: HistoryHorizon::All, ..Default::default() },
        );
    }
    if only.is_empty() || only == "window" {
        run(
            "window",
            HistoryOptions {
                horizon: HistoryHorizon::Window(Duration::from_secs(2)),
                cap_bytes: 4 * 1024 * 1024,
            },
        );
    }
    if only.is_empty() || only == "trigger" {
        // Same window retention + `auto_compact_sst_count=8`: measured
        // NEGATIVE — the count path still promotes one level at a time
        // (lowest first, so L0 wins) and never revisits aged versions that
        // already reached L1+. Kept to document the swept lever.
        let mut opts = OpenOptions::default();
        opts.history = HistoryOptions {
            horizon: HistoryHorizon::Window(Duration::from_secs(2)),
            cap_bytes: 4 * 1024 * 1024,
        };
        run_opts("trigger", opts);
    }
    println!("SIZING-AB-DONE");
}
