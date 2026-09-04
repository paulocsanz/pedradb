//! First-class CAS: the primitive leases and leader locks are built on.
//!
//! `put_if_absent` / `put_if_eq` / `compare_and_swap` fail closed with
//! `CasMismatch` — no silent overwrite, no DIY get-then-put race. A single
//! writer is enough to see the shape; `concurrent` shows many threads.
//!
//! ```sh
//! cargo run -p pedradb-examples --example cas
//! ```
//!
//! Next: `snapshots` — read the past without copying the database.

use pedradb_core::{CoreError, Db};

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
    let dir = scratch("cas");
    let mut db = Db::open(&dir)?;

    // Acquire a free lease (IF NOT EXISTS).
    db.put_if_absent(b"lease/volume-1", b"node-a")?;
    assert_eq!(
        db.get(b"lease/volume-1").as_deref(),
        Some(b"node-a".as_ref())
    );

    // Second acquirer loses. The key is unchanged.
    match db.put_if_absent(b"lease/volume-1", b"node-b") {
        Err(CoreError::CasMismatch) => {}
        other => panic!("expected CasMismatch, got {other:?}"),
    }
    assert_eq!(
        db.get(b"lease/volume-1").as_deref(),
        Some(b"node-a".as_ref())
    );

    // Transfer only if we still hold it (fencing: a stale leader cannot steal back
    // after a newer holder has CAS'd).
    db.compare_and_swap(b"lease/volume-1", b"node-a", b"node-c")?;
    assert_eq!(
        db.get(b"lease/volume-1").as_deref(),
        Some(b"node-c".as_ref())
    );

    match db.put_if_eq(b"lease/volume-1", b"node-a", b"node-a") {
        Err(CoreError::CasMismatch) => {}
        other => panic!("stale holder must not win, got {other:?}"),
    }

    // Release: CAS back to empty via delete after an eq check, in one writer.
    db.delete(b"lease/volume-1")?;
    db.put_if_absent(b"lease/volume-1", b"node-b")?;
    assert_eq!(
        db.get(b"lease/volume-1").as_deref(),
        Some(b"node-b".as_ref())
    );

    println!("cas: lease/volume-1 held by node-b (absent / eq / stale fenced)");
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
        super::run().expect("cas");
    }
}
