//! RFC-0047 telemetry: retention-profile A/B + PointInTime RecoveryReport
//! print (acceptance criteria "Telemetry").
//!
//! Run:
//!   cargo run -q --release -p rocksdb-compat --example rfc0047_profile_ab
//!
//! Output → findings/rfc0047-profile-ab/ (README.md curates the numbers).

#![forbid(unsafe_code)]

use rocksdb_compat::{Options, DB};
use std::path::Path;
use std::time::Instant;

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            total += dir_size(&p);
        } else {
            total += std::fs::metadata(&p).unwrap().len();
        }
    }
    total
}

fn tmp_under(root: &Path, tag: &str) -> std::path::PathBuf {
    let dir = root.join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Same overwrite workload as `auto_reclaim_default_matches_rocks_profile`
/// (20 rounds × 10 keys × 1 KiB incompressible, per-round flush, 64 KiB
/// buffer): retention divergence, not live-set growth. Adds a full-scan
/// wall-time measurement on a fresh reopen.
fn profile_round(root: &Path, tag: &str, reclaim: bool) -> (u64, u128, usize) {
    let dir = tmp_under(root, tag);
    let mut opts = Options::new();
    opts.create_if_missing(true);
    opts.write_buffer_size = 64 * 1024;
    opts.auto_reclaim = reclaim;
    {
        let db = DB::open_cf_with_env(&opts, &dir, &[], pedradb_core::StdEnv).unwrap();
        let mut seed = 0x5EED_0047_u64;
        let mut value = vec![0u8; 1024];
        for _round in 0..20u64 {
            for byte in value.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            for i in 0..10 {
                db.put(format!("k{i:02}").as_bytes(), &value).unwrap();
            }
            db.flush().unwrap();
        }
    }
    let size = dir_size(&dir);
    // Fresh reopen (closed handle first): full-scan wall time.
    let db = DB::open_cf_with_env(&opts, &dir, &[], pedradb_core::StdEnv).unwrap();
    let t0 = Instant::now();
    let mut it = db.iterator(rocksdb_compat::IteratorMode::Start).unwrap();
    let mut n = 0usize;
    while it.valid() {
        n += 1;
        it.next();
    }
    let scan_us = t0.elapsed().as_micros();
    drop(it);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    (size, scan_us, n)
}

/// Corrupt a mid-WAL record (FlipCrc #3 of 8) and reopen on the drop-in
/// default (PointInTime): the prefix is served and the discard reported.
fn recovery_print(root: &Path) {
    use pedradb_core::wal::recover_choose::{apply_recover_choice, RecoverChoice};

    let dir = tmp_under(root, "pit-report");
    let mut opts = Options::new();
    opts.create_if_missing(true);
    {
        let db = DB::open(&opts, &dir).unwrap();
        for i in 0..8u32 {
            db.put(format!("k{i:02}").as_bytes(), &[7u8; 120]).unwrap();
        }
    }
    let wal = dir.join(pedradb_core::WAL_FILE_NAME);
    let mut bytes = std::fs::read(&wal).unwrap();
    assert!(apply_recover_choice(
        &mut bytes,
        RecoverChoice::FlipCrc { index: 3 }
    ));
    std::fs::write(&wal, &bytes).unwrap();

    let db = DB::open(&opts, &dir).unwrap();
    let report = db.last_recovery_report().expect("PointInTime must report");
    println!("recovery_report: kind={}", report.kind);
    println!(
        "recovery_report: corrupt_offset={} good_through_offset={} discarded_bytes={}",
        report.corrupt_offset, report.good_through_offset, report.discarded_bytes
    );
    let prefix = db.get(b"k02").unwrap().is_some();
    let suffix = db.get(b"k03").unwrap().is_some();
    println!(
        "recovery_report: prefix k02 visible={} suffix k03 discarded={}",
        prefix, !suffix
    );
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

fn main() {
    let root = std::path::PathBuf::from("findings/rfc0047-profile-ab/scratch");
    std::fs::create_dir_all(&root).unwrap();

    let (reclaim_size, reclaim_scan, reclaim_n) = profile_round(&root, "reclaim", true);
    let (f20_size, f20_scan, f20_n) = profile_round(&root, "f20", false);
    println!("profile_ab: live_set_bytes=10240 written_bytes=204800");
    println!(
        "profile_ab: auto_reclaim=true  final_disk={}B  full_scan={}us  keys={}",
        reclaim_size, reclaim_scan, reclaim_n
    );
    println!(
        "profile_ab: auto_reclaim=false final_disk={}B  full_scan={}us  keys={}",
        f20_size, f20_scan, f20_n
    );
    println!(
        "profile_ab: disk_ratio_f20_over_reclaim={:.1}x",
        f20_size as f64 / reclaim_size as f64
    );

    recovery_print(&root);
    let _ = std::fs::remove_dir_all(&root);
}
