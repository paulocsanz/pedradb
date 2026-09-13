//! RFC-0193 P0.1: deterministic WAL byte-identity capture.
//!
//! Drives the real `ConcurrentDb` write paths single-threaded with a fixed
//! op sequence (async 1c pipeline puts, G1 sync puts, deletes, v2-interned
//! same-value runs, a block-spanning value, close + reopen-append) and
//! copies the live WAL segment to `<out-dir>/CURRENT.log`.
//!
//! Bytes are a pure function of the op sequence (seqs are allocated in
//! order, one thread) — running this before and after a write-path change
//! and `cmp`-ing the two files is the RFC-0193 byte-identity gate.
//!
//! ```text
//! cargo run -q -p pedradb-core --example wal_capture -- <out-dir>
//! ```

use std::path::Path;

fn main() {
    let out = std::env::args()
        .nth(1)
        .expect("usage: wal_capture <out-dir>");
    let work = std::env::temp_dir().join(format!("wal-capture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let opts = |sync: bool| pedradb_core::db::OpenOptions {
        wal_full_fsync: true,
        history: Default::default(),
        wal_recovery: Default::default(),
        sync,
        auto_flush_bytes: None,
        ..Default::default()
    };

    {
        let db = pedradb_core::concurrent::ConcurrentDb::open_with(&work, opts(false)).unwrap();
        // Async 1c pipeline path (group of one).
        for i in 0..8u32 {
            db.put(format!("async/{i:03}"), format!("v-{i:03}")).unwrap();
        }
        // G1/verified group path: write + fdatasync before Ok.
        for i in 0..5u32 {
            db.put_with(
                format!("g1/{i:03}"),
                format!("g-{i:03}"),
                pedradb_core::db::WriteOptions {
                    sync: Some(true),
                },
            )
            .unwrap();
        }
        // v2 interning: consecutive same-value puts share one payload.
        let shared = vec![0x5au8; 120];
        for i in 0..20u32 {
            db.put(format!("intern/{i:03}"), shared.clone()).unwrap();
        }
        // Delete (async) after the G1 records.
        db.delete("async/000").unwrap();
        // One record spanning several 32 KiB blocks (First/Middle/Last).
        let big = vec![0xa5u8; 40_000];
        db.put_with("big/000", big, pedradb_core::db::WriteOptions::default())
            .unwrap();
        // Second G1 record after the spanning one.
        db.put_with(
            "g1/005",
            "tail",
            pedradb_core::db::WriteOptions {
                sync: Some(true),
            },
        )
        .unwrap();
    }
    // Close + reopen: append path initializes from the existing segment.
    {
        let db = pedradb_core::concurrent::ConcurrentDb::open_with(&work, opts(true)).unwrap();
        for i in 0..4u32 {
            db.put(format!("reopen/{i:03}"), format!("r-{i:03}")).unwrap();
        }
        db.delete("g1/002").unwrap();
    }

    let wal = work.join(pedradb_core::db::WAL_FILE_NAME);
    let bytes = std::fs::read(&wal).expect("wal segment exists");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(Path::new(&out).join("wal-before.bin"), &bytes).unwrap();
    println!(
        "wal_capture: {} bytes -> {}/wal-before.bin",
        bytes.len(),
        out
    );
    let _ = std::fs::remove_dir_all(&work);
}
