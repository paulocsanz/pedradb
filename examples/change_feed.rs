//! A durable change feed: watch / CDC without another system.
//!
//! After `commit` returns `Ok`, `changes(from, to)` and `changes_after(from)`
//! replay the logical puts and deletes in sequence order. Catch up from the
//! last sequence you were served — the feed is the same WAL the engine
//! already fsynced.
//!
//! ```sh
//! cargo run -p pedradb-examples --example change_feed
//! ```
//!
//! Next: `crash_reopen` — kill the process after Ok; reopen; the keys remain.

use pedradb_core::{ChangeKind, Db};

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
    let dir = scratch("feed");
    let mut db = Db::open(&dir)?;

    let cursor = db.last_sequence();
    db.put(b"user/ada", b"active")?;
    db.put(b"user/bob", b"active")?;
    db.delete(b"user/bob")?;
    let head = db.last_sequence();

    // Windowed feed: (cursor, head] — exact, fails closed if GC aged it out.
    let window = db.changes(cursor, head)?;
    assert_eq!(window.len(), 3);
    assert_eq!(window[0].kind, ChangeKind::Put);
    assert_eq!(window[0].key.as_ref(), b"user/ada");
    assert_eq!(window[1].kind, ChangeKind::Put);
    assert_eq!(window[2].kind, ChangeKind::Delete);
    assert_eq!(window[2].key.as_ref(), b"user/bob");

    // Tail: everything after `head` (empty until the next durable commit).
    assert!(db.changes_after(head).is_empty());
    db.put(b"user/cy", b"active")?;
    let tail = db.changes_after(head);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].key.as_ref(), b"user/cy");

    println!(
        "change_feed: {} events in window, {} on tail (ada put, bob delete, cy put)",
        window.len(),
        tail.len()
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
        super::run().expect("change_feed");
    }
}
