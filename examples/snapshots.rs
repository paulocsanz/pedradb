//! MVCC: write a new version, keep reading the old one.
//!
//! Every commit is a sequence number. `get_at(Snapshot::at(seq), key)` is the
//! value as of that commit — the same primitive a snapshot isolation layer,
//! a backup, or a "what did the user see?" debug path needs. Pin the snapshot
//! if you will compact while it is still live.
//!
//! ```sh
//! cargo run -p pedradb-examples --example snapshots
//! ```
//!
//! Next: `change_feed` — the same sequences, as a stream of puts and deletes.

use pedradb_core::{Db, Snapshot};

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
    let dir = scratch("snap");
    let mut db = Db::open(&dir)?;

    db.put(b"doc", b"v1")?;
    let seq_v1 = db.last_sequence();
    let pin = db.pin_snapshot();

    db.put(b"doc", b"v2")?;
    let seq_v2 = db.last_sequence();
    db.put(b"doc", b"v3")?;

    assert_eq!(db.get(b"doc").as_deref(), Some(b"v3".as_ref()));
    assert_eq!(
        db.get_at(Snapshot::at(seq_v1), b"doc")?.as_deref(),
        Some(b"v1".as_ref())
    );
    assert_eq!(
        db.get_at(Snapshot::at(seq_v2), b"doc")?.as_deref(),
        Some(b"v2".as_ref())
    );
    assert_eq!(
        db.get_at(pin.snapshot(), b"doc")?.as_deref(),
        Some(b"v1".as_ref())
    );

    // Point-in-time over several keys: multi_get_at is the same visibility.
    db.put(b"meta", b"m3")?;
    let both = db.multi_get_at(Snapshot::at(seq_v2), &[b"doc".as_ref(), b"meta".as_ref()])?;
    assert_eq!(both[0].as_deref(), Some(b"v2".as_ref()));
    assert!(both[1].is_none(), "meta did not exist at seq_v2");

    db.release_snapshot_pin(pin);
    println!("snapshots: live=v3  @v1=v1  @v2=v2  (pin held across overwrites)");
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
        super::run().expect("snapshots");
    }
}
