//! Keys are bytes. Scan is lexicographic. Pagination is a limit, not a Vec.
//!
//! `user/1`, `user/10`, `user/2` sort as 1, 10, 2 — pad (or encode a big-endian
//! integer) if you want numeric order. Prefix scans use
//! [`pedradb_core::prefix_exclusive_end`], never `prefix || 0xff`.
//!
//! ```sh
//! cargo run -p pedradb-examples --example ordered_scan
//! ```
//!
//! Next: `cas` — first-class compare-and-swap (leases, leader locks).

use std::ops::Bound;

use pedradb_core::{prefix_exclusive_end, Db, ScanProjection};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run() -> pedradb_core::Result<()> {
    let dir = scratch("scan");
    let mut db = Db::open(&dir)?;

    // Naive ids: lexicographic order is not numeric order.
    for (k, v) in [
        (b"user/1" as &[u8], b"ada" as &[u8]),
        (b"user/2", b"bob"),
        (b"user/10", b"cy"),
    ] {
        db.put(k, v)?;
    }
    // A neighbouring prefix must not leak into a `user/` scan.
    db.put(b"user0/x", b"nope")?;
    db.put(b"z/tail", b"nope")?;

    let user_end = prefix_exclusive_end(b"user/");
    let naive: Vec<_> = db
        .scan(
            Bound::Included(b"user/".as_ref()),
            user_end
                .as_deref()
                .map_or(Bound::Unbounded, Bound::Excluded),
        )
        .map(|kv| String::from_utf8_lossy(&kv.key).into_owned())
        .collect();
    assert_eq!(naive, ["user/1", "user/10", "user/2"]);

    // Zero-padded ids scan in numeric order.
    for i in [1u32, 2, 10] {
        db.put(format!("acct/{i:04}").as_bytes(), b"ok")?;
    }
    let acct_end = prefix_exclusive_end(b"acct/");
    let padded: Vec<_> = db
        .scan(
            Bound::Included(b"acct/".as_ref()),
            acct_end
                .as_deref()
                .map_or(Bound::Unbounded, Bound::Excluded),
        )
        .map(|kv| String::from_utf8_lossy(&kv.key).into_owned())
        .collect();
    assert_eq!(padded, ["acct/0001", "acct/0002", "acct/0010"]);

    // Pagination: stop after N live keys (do not collect the world).
    let page = db.range_limited(
        Bound::Included(b"acct/".as_ref()),
        Bound::Excluded(b"acct0".as_ref()),
        Some(2),
    );
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].0.as_ref(), b"acct/0001");

    // Key-only projection skips loading values (listings, counts).
    let names: Vec<_> = db
        .scan_projected(
            Bound::Included(b"user/".as_ref()),
            Bound::Excluded(b"user0".as_ref()),
            ScanProjection::KeyOnly,
        )
        .map(|kv| {
            assert!(kv.value.is_empty());
            kv.key
        })
        .collect();
    assert_eq!(names.len(), 3);

    println!(
        "ordered_scan: naive={naive:?} padded={padded:?} page={}",
        page.len()
    );
    db.close()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> pedradb_core::Result<()> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("ordered_scan");
    }
}
