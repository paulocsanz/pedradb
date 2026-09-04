//! The rust-rocksdb 0.22 surface, on PedraDB.
//!
//! Point `rocksdb` at `rocksdb-compat` and keep `DB::open_default` / `put` /
//! `get` / `WriteBatch` / `iterator`. Semantics that would be silent-wrong in
//! a naive translation (skip-any WAL, dropping SST files without tombstones)
//! stay Pedra-correct — see `docs/rocksdb-compat.md`.
//!
//! ```sh
//! cargo run -p pedradb-examples --example rocksdb_dropin
//! ```

use rocksdb_compat::{IteratorMode, WriteBatch, DB};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn run() -> Result<(), rocksdb_compat::Error> {
    let dir = scratch("rocks");
    let db = DB::open_default(&dir)?;

    db.put(b"hello", b"world")?;
    assert_eq!(db.get(b"hello")?, Some(b"world".to_vec()));

    let mut batch = WriteBatch::new();
    batch.put(b"a", b"1");
    batch.put(b"b", b"2");
    batch.delete(b"hello");
    db.write(&batch)?;
    assert!(db.get(b"hello")?.is_none());

    let mut keys = Vec::new();
    for item in db.iterator(IteratorMode::Start)? {
        let (k, v) = item?;
        keys.push(format!(
            "{}={}",
            String::from_utf8_lossy(&k),
            String::from_utf8_lossy(&v)
        ));
    }
    assert_eq!(keys, ["a=1", "b=2"]);

    println!("rocksdb_dropin: WriteBatch put a,b + delete hello; scan={keys:?}");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> Result<(), rocksdb_compat::Error> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("rocksdb_dropin");
    }
}
