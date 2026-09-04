//! Checkpoint, ship WAL, restore to a point in time.
//!
//! `BackupEngine` is local ops on one directory — not a cluster backup. Take a
//! base (openable as a DB), keep shipping complete WAL records, restore with
//! `restore_pitr` up to a sequence. Same split as Postgres base + WAL.
//!
//! ```sh
//! cargo run -p pedradb-examples --example backup
//! ```

use pedradb_core::{Db, StdEnv};
use pedradb_ops::BackupEngine;

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run() -> pedradb_ops::Result<()> {
    let root = scratch("backup");
    let live = root.join("live");
    let backups = root.join("backups");
    let restored = root.join("restored");

    let mut db = Db::open(&live)?;
    db.put(b"epoch", b"base")?;
    db.put(b"k", b"v1")?;

    let mut engine = BackupEngine::open_with_env(&backups, StdEnv)?;
    let base = engine.create_base_backup(&mut db)?;

    db.put(b"k", b"v2")?;
    db.put(b"later", b"yes")?;
    let shipped = engine.ship_wal(&db)?;

    engine.restore_pitr(base.id, &restored, Some(shipped.last_shipped_sequence))?;

    let replayed = Db::open(&restored)?;
    assert_eq!(replayed.get(b"epoch").as_deref(), Some(b"base".as_ref()));
    assert_eq!(replayed.get(b"k").as_deref(), Some(b"v2".as_ref()));
    assert_eq!(replayed.get(b"later").as_deref(), Some(b"yes".as_ref()));

    // Base-only restore (no WAL replay) still has the checkpoint contents.
    let base_only = root.join("base-only");
    engine.restore(base.id, &base_only)?;
    let frozen = Db::open(&base_only)?;
    assert_eq!(frozen.get(b"k").as_deref(), Some(b"v1".as_ref()));
    assert!(frozen.get(b"later").is_none());

    println!(
        "backup: base seq={}  pitr has k=v2+later  base-only has k=v1",
        base.base_sequence
    );
    replayed.close()?;
    frozen.close()?;
    db.close()?;
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

fn main() -> pedradb_ops::Result<()> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("backup");
    }
}
