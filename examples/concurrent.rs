//! Many threads, one engine: group commit, OCC, and CAS — each where it fits.
//!
//! `ConcurrentDb` is `Clone` + `Sync`. Writers join a group — one `fdatasync`
//! covers everyone who arrived while the leader was encoding. Readers never
//! take the write lock.
//!
//! OCC (`begin_occ`) is snapshot isolation: disjoint keys commit together.
//! Members of the *same* write group do not conflict with each other, so a
//! hot counter is `put_if_eq`, not OCC — otherwise two increments in one
//! fsync would both succeed and one add would vanish.
//!
//! ```sh
//! cargo run -p pedradb-examples --example concurrent
//! ```
//!
//! Next: `bank` — the pieces so far, as one small ledger.

use std::sync::{Arc, Barrier};
use std::thread;

use pedradb_core::{ConcurrentDb, CoreError};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn parse_u64(bytes: &[u8]) -> u64 {
    std::str::from_utf8(bytes)
        .expect("utf-8")
        .parse()
        .expect("u64")
}

fn occ_move(db: &ConcurrentDb, from: &[u8], to: &[u8]) -> pedradb_core::Result<()> {
    loop {
        let mut tx = db.begin_occ();
        let src = parse_u64(&tx.get(from)?.expect("src"));
        let dst = parse_u64(&tx.get(to)?.expect("dst"));
        tx.put(from, src.saturating_sub(1).to_string())?;
        tx.put(to, (dst + 1).to_string())?;
        match tx.commit() {
            Ok(()) => return Ok(()),
            Err(CoreError::TransactionConflict) => continue,
            Err(e) => return Err(e),
        }
    }
}

fn cas_bump(db: &ConcurrentDb) -> pedradb_core::Result<()> {
    loop {
        let cur = db.get(b"counter").expect("seeded");
        let next = (parse_u64(&cur) + 1).to_string();
        match db.put_if_eq(b"counter", cur.as_ref(), next.as_bytes()) {
            Ok(_) => return Ok(()),
            Err(CoreError::CasMismatch) => continue,
            Err(e) => return Err(e),
        }
    }
}

fn run() -> pedradb_core::Result<()> {
    let dir = scratch("conc");
    let db = ConcurrentDb::open(&dir)?;

    // Distinct keys: group commit amortizes the WAL barrier.
    let workers = 4;
    let puts_each = 16;
    let barrier = Arc::new(Barrier::new(workers));
    let mut joins = Vec::new();
    for t in 0..workers {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        joins.push(thread::spawn(move || {
            barrier.wait();
            for i in 0..puts_each {
                let key = format!("t{t}/{i:02}");
                db.put(key.as_bytes(), b"v").expect("put");
            }
        }));
    }
    for j in joins {
        j.join().expect("worker");
    }

    let (submits, queued, groups, ops) = db.write_group_stats();
    let keys: [&[u8]; 2] = [b"t0/00", b"t3/15"];
    let got = db.multi_get(&keys);
    assert_eq!(got[0].as_deref(), Some(b"v".as_ref()));
    assert_eq!(got[1].as_deref(), Some(b"v".as_ref()));

    // OCC on disjoint pairs: both moves are valid even if they share a group.
    db.put(b"ada", b"5")?;
    db.put(b"bob", b"0")?;
    db.put(b"cy", b"5")?;
    db.put(b"dan", b"0")?;
    {
        let a = db.clone();
        let b = db.clone();
        let left = thread::spawn(move || occ_move(&a, b"ada", b"bob").expect("ada→bob"));
        let right = thread::spawn(move || occ_move(&b, b"cy", b"dan").expect("cy→dan"));
        left.join().expect("left");
        right.join().expect("right");
    }
    assert_eq!(parse_u64(&db.get(b"ada").expect("ada")), 4);
    assert_eq!(parse_u64(&db.get(b"bob").expect("bob")), 1);
    assert_eq!(parse_u64(&db.get(b"cy").expect("cy")), 4);
    assert_eq!(parse_u64(&db.get(b"dan").expect("dan")), 1);

    // Hot key: CAS. Every increment is a verified successor of the value it read.
    db.put(b"counter", b"0")?;
    let bumps = 8;
    let barrier = Arc::new(Barrier::new(workers));
    let mut joins = Vec::new();
    for _ in 0..workers {
        let db = db.clone();
        let barrier = Arc::clone(&barrier);
        joins.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..bumps {
                cas_bump(&db).expect("bump");
            }
        }));
    }
    for j in joins {
        j.join().expect("bumper");
    }
    let total = parse_u64(&db.get(b"counter").expect("counter"));
    assert_eq!(total, (workers * bumps) as u64);

    println!(
        "concurrent: {} puts in {groups} groups (submits={submits} queued={queued} ops={ops}); \
         OCC ada/bob + cy/dan; CAS counter={total}",
        workers * puts_each
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
        super::run().expect("concurrent");
    }
}
