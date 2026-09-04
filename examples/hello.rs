//! The smallest useful PedraDB program.
//!
//! Open a directory, put a key, get it back. Close the handle, open the same
//! directory again — the write is still there. That is the whole product at
//! rest: an embeddable library, durable by default, no server.
//!
//! ```sh
//! cargo run -p pedradb-examples --example hello
//! ```
//!
//! Next: `transactions` — several keys in one atomic commit.

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
    let dir = scratch("hello");

    {
        let mut db = Db::open(&dir)?;
        db.put(b"hello", b"world")?;
        assert_eq!(db.get(b"hello").as_deref(), Some(b"world".as_ref()));
        db.close()?;
    }

    // Process "exited". Reopen the same directory — WAL replay restores the key.
    let db = Db::open(&dir)?;
    assert_eq!(db.get(b"hello").as_deref(), Some(b"world".as_ref()));
    println!("hello: reopen saw hello=world (seq={})", db.last_sequence());
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
        super::run().expect("hello");
    }
}
