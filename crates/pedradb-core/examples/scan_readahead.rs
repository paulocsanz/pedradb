//! RFC-0195 P0.3 — scan-readahead telemetry on a real scanning workload
//! (real files, real block walk, default `StdEnv`).
//!
//! Two legs, same workload (~320 KB across ~80 data blocks, flushed to
//! one SST, then fully scanned):
//! * bounded-cache leg (payload budget 1 B, warm cap 1 B): the store is
//!   bounded and the SST serves from file, so the sequential walk issues
//!   `WILLNEED` windows ahead of the cursor — the counters must move;
//! * fitting leg (default budget, warm cap `u64::MAX`): condition (a)
//!   fails by construction — the line must read all zeros.
//!
//! The telemetry line is the shipped opt-in one (`PEDRA_IO_ADVISE_STATS`,
//! read at call time): the example first shows it absent with the latch
//! unset, then arms the latch and prints it for both legs.
//!
//! Usage: `cargo run --release -p pedradb-core --example scan_readahead`

use pedradb_core::env::StdEnv;
use pedradb_core::{ConcurrentDb, OpenOptions};
use std::fs;
use std::ops::Bound;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "pedra-scan-readahead-{tag}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// The `scan_readahead_fires_only_in_bounded_cache` shape on real files:
/// seed enough rows to span dozens of 4 KiB data blocks, flush to one
/// SST, then walk the whole range — a long sequential block walk.
fn drive(db: &ConcurrentDb) -> usize {
    let val = vec![b'v'; 200];
    for i in 0..1600u32 {
        db.put(format!("sr/{i:05}").as_bytes(), &val).unwrap();
    }
    db.flush().unwrap();
    db.scan_collect(Bound::<&[u8]>::Unbounded, Bound::<&[u8]>::Unbounded)
        .len()
}

fn main() {
    // Bounded-cache leg: bounded ∧ file-served ⇒ the walk reads ahead.
    let dir_bounded = tmp_dir("bounded");
    let db = ConcurrentDb::open_with_env_bounded(
        &dir_bounded,
        OpenOptions {
            sst_payload_budget_bytes: Some(1),
            sst_warm_cap_bytes: 1,
            auto_flush_bytes: None,
            ..OpenOptions::default()
        },
        StdEnv,
    )
    .unwrap();
    let rows = drive(&db);
    println!("bounded leg scanned {rows} rows");
    println!("bounded leg line (latch unset): {:?}", db.io_advise_line());
    std::env::set_var("PEDRA_IO_ADVISE_STATS", "1");
    println!(
        "bounded leg line (latch on):    {}",
        db.io_advise_line().unwrap_or_else(|| "ABSENT".into())
    );
    drop(db);
    let _ = fs::remove_dir_all(&dir_bounded);

    // Fitting leg: warm cap MAX ⇒ condition (a) fails — counters stay zero.
    let dir_fitting = tmp_dir("fitting");
    let db = ConcurrentDb::open_with_env_bounded(
        &dir_fitting,
        OpenOptions {
            sst_warm_cap_bytes: u64::MAX,
            auto_flush_bytes: None,
            ..OpenOptions::default()
        },
        StdEnv,
    )
    .unwrap();
    let rows = drive(&db);
    println!("fitting leg scanned {rows} rows");
    println!(
        "fitting leg line:               {}",
        db.io_advise_line().unwrap_or_else(|| "ABSENT".into())
    );
    drop(db);
    let _ = fs::remove_dir_all(&dir_fitting);
}
