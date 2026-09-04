//! Opt-in value log: large blobs stay out of the LSM.
//!
//! Default is inline (safe). Set `large_value_threshold` when you have measured
//! that big values are amplifying SST rewrite. After overwrite/delete churn,
//! compact old SST versions first, then `compact_vlog` to reclaim the log.
//!
//! ```sh
//! cargo run -p pedradb-examples --example large_values
//! ```

use pedradb_core::{CompactOptions, Db, OpenOptions};

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
    let dir = scratch("vlog");
    let mut db = Db::open_with(
        &dir,
        OpenOptions {
            large_value_threshold: Some(1024),
            ..OpenOptions::default()
        },
    )?;

    let v1 = vec![1u8; 8 * 1024];
    let v2 = vec![2u8; 8 * 1024];
    db.put(b"blob", &v1)?;
    assert_eq!(db.get(b"blob").as_deref(), Some(v1.as_slice()));

    db.put(b"blob", &v2)?;
    assert_eq!(db.get(b"blob").as_deref(), Some(v2.as_slice()));

    db.flush()?;
    db.compact_with(CompactOptions::latest_only())?;
    let stats = db.compact_vlog()?;
    assert_eq!(db.get(b"blob").as_deref(), Some(v2.as_slice()));

    println!(
        "large_values: vlog {}B → {}B ({} live)  {}",
        stats.bytes_before,
        stats.bytes_after,
        stats.live_records,
        db.stats().vlog_line()
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
        super::run().expect("large_values");
    }
}
