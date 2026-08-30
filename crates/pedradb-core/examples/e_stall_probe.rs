//! ycsb_e version-retention cliff probe (RFC-0044 P2.2). Diagnostic for
//! `findings/rfc0044-p1/ycsb-longwindow/`: hot-key version piles make cold
//! `count_in_range` walks O(retained versions); `compact_for_reads` (GC)
//! collapses them.
//!
//! Usage: `cargo run --release --example e_stall_probe -- <db-dir>`

use std::ops::Bound;
use std::time::Instant;

use pedradb_core::Db;

fn ykey(i: usize) -> Vec<u8> {
    format!("ycsb/{i:06}").into_bytes()
}

/// Window end exactly as the bench builds it: ykey(i+25), last byte -> '~'.
fn window_end(i: usize) -> Vec<u8> {
    let mut e = ykey(i + 25);
    e.pop();
    e.push(b'~');
    e
}

fn time_scan(db: &Db, i: usize, reps: usize) {
    let start = ykey(i);
    let end = window_end(i);
    for r in 0..reps {
        let t = Instant::now();
        let n = db.count_in_range(
            db.visible_sequence(),
            Bound::Included(start.as_slice()),
            Bound::Excluded(end.as_slice()),
            Some(25),
        );
        let d = t.elapsed();
        println!(
            "scan i={i:4} rep{r}: n={n:?} {:.3} ms",
            d.as_secs_f64() * 1e3
        );
        if d.as_secs_f64() > 5.0 && r + 1 < reps {
            println!("scan i={i}: too slow, skipping remaining reps");
            break;
        }
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: e_stall_probe <db-dir>");
    let t = Instant::now();
    let mut db = Db::open(&dir).expect("open");
    println!("open+wal-replay: {:.1}s", t.elapsed().as_secs_f64());

    println!("--- before GC ---");
    time_scan(&db, 0, 3); // zipf-hottest window (ycsb/000000..000025)
    time_scan(&db, 1, 2);
    time_scan(&db, 10, 2);
    time_scan(&db, 500, 2); // cold window
    let t = Instant::now();
    let got = db.get(b"ycsb/000000");
    println!(
        "point get hottest key: {:.1} us (some={})",
        t.elapsed().as_secs_f64() * 1e6,
        got.is_some()
    );

    println!("--- compact_for_reads (latest-only GC) ---");
    let t = Instant::now();
    db.compact_for_reads().expect("compact");
    println!("compact_for_reads: {:.1}s", t.elapsed().as_secs_f64());

    println!("--- after GC ---");
    time_scan(&db, 0, 5);
    time_scan(&db, 1, 2);
    time_scan(&db, 500, 2);
    let t = Instant::now();
    let got = db.get(b"ycsb/000000");
    println!(
        "point get hottest key: {:.1} us (some={})",
        t.elapsed().as_secs_f64() * 1e6,
        got.is_some()
    );
}
