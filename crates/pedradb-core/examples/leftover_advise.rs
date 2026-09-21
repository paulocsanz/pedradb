//! RFC-0194 P0.3 — leftover page-advise telemetry on a real compacting
//! workload (real files, real flush worker, default `StdEnv`).
//!
//! Two legs, same workload:
//! * bounded-cache leg (`sst_page_keep_budget: 0`, warm cap 1 B): the
//!   Fire-119 condition holds, so leftover SST consumption queues
//!   DONTNEED and the drain issues it — counters must move;
//! * default leg (budget `u64::MAX`): the policy is off — the line must
//!   read all zeros (zero bookkeeping on the default path).
//!
//! The telemetry line is the shipped opt-in one (`PEDRA_IO_ADVISE_STATS`,
//! read at call time): the example first shows it absent with the latch
//! unset, then arms the latch and prints it for both legs.
//!
//! Usage: `cargo run --release -p pedradb-core --example leftover_advise`

use pedradb_core::{ConcurrentDb, OpenOptions};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "pedra-leftover-advise-{tag}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// The `leftover_advise_fires_only_in_bounded_cache` shape on real files:
/// seed one-slash families **in the same memtable** as the fresh family,
/// so the flush materializes the seeded families as leftovers with no
/// prior live SST covering them (a pre-seeded flush would make them live
/// and the covering rule correctly skips them). A second round then
/// exercises the covered leg on the now-live families.
fn drive(db: &ConcurrentDb, n: usize) {
    for i in 0..n {
        db.put(format!("ycsb/{i}").as_bytes(), &[b'v'; 64]).unwrap();
    }
    // Fresh family in the SAME memtable: its flush parks/materializes the
    // seeded families as leftovers — the Drop leg.
    db.put(b"c/1", &[b'x'; 64]).unwrap();
    db.flush().unwrap();
    // Second round: ycsb/ is now live — a new leftover of the same family
    // is covered (KeepCovered leg).
    db.put(b"ycsb/9", &[b'v'; 64]).unwrap();
    db.put(b"c/2", &[b'x'; 64]).unwrap();
    db.flush().unwrap();
}

fn main() {
    // Bounded-cache leg: the Fire-119 condition holds by construction.
    let dir_bounded = tmp_dir("bounded");
    let db = ConcurrentDb::open_with(
        &dir_bounded,
        OpenOptions {
            auto_flush_bytes: None,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    db.set_sst_page_keep_budget(0);
    db.set_sst_warm_cap_bytes(1);
    drive(&db, 6);
    println!("bounded leg line (latch unset): {:?}", db.io_advise_line());
    std::env::set_var("PEDRA_IO_ADVISE_STATS", "1");
    println!("bounded leg line (latch on):    {}", db.io_advise_line().unwrap_or_else(|| "ABSENT".into()));
    drop(db);
    let _ = fs::remove_dir_all(&dir_bounded);

    // Default leg: policy off — counters stay zero.
    let dir_default = tmp_dir("default");
    let db = ConcurrentDb::open_with(&dir_default, OpenOptions::default()).unwrap();
    drive(&db, 6);
    println!("default leg line:              {}", db.io_advise_line().unwrap_or_else(|| "ABSENT".into()));
    drop(db);
    let _ = fs::remove_dir_all(&dir_default);
}
