//! Durability contract, in two directories.
//!
//! 1. `commit` / `put` returned `Ok` → kill the handle (no `close`) → reopen
//!    the same directory → the keys are there. WAL replay is the recovery path.
//! 2. A transaction that never committed is gone. A crash mid-write cannot
//!    leave a partial multi-key update.
//!
//! This is a process-exit simulation, not a plug-pull. The default open still
//! `fdatasync`s (and `F_FULLFSYNC` on Darwin) before Ok.
//!
//! ```sh
//! cargo run -p pedradb-examples --example crash_reopen
//! ```
//!
//! Next: `concurrent` — many threads, one fsync per group, OCC on conflict.

use pedradb_core::Db;

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
    let dir = scratch("crash");

    {
        let mut db = Db::open(&dir)?;
        let mut tx = db.begin();
        tx.put(b"row/42", br#"{"name":"ada"}"#)?;
        tx.put(b"idx/ada", b"42")?;
        tx.commit()?;
        // No close: Drop releases the directory lock the way a process exit would.
    }

    {
        let db = Db::open(&dir)?;
        assert_eq!(
            db.get(b"row/42").as_deref(),
            Some(br#"{"name":"ada"}"#.as_ref())
        );
        assert_eq!(db.get(b"idx/ada").as_deref(), Some(b"42".as_ref()));
        db.close()?;
    }

    // Uncommitted staging never reaches the WAL.
    {
        let mut db = Db::open(&dir)?;
        let mut tx = db.begin();
        tx.put(b"row/99", b"ghost")?;
        tx.put(b"idx/ghost", b"99")?;
        tx.abort();
        db.close()?;
    }
    {
        let db = Db::open(&dir)?;
        assert!(db.get(b"row/99").is_none());
        assert!(db.get(b"idx/ghost").is_none());
        println!(
            "crash_reopen: committed row+idx survived drop; aborted pair did not (seq={})",
            db.last_sequence()
        );
        db.close()?;
    }

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
        super::run().expect("crash_reopen");
    }
}
