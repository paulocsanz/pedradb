//! Multi-key ACID: debit A, credit B, write an audit row — or none of them.
//!
//! RocksDB will take each `put` independently. PedraDB's `begin` / `commit`
//! writes one WAL record for the whole batch; `commit` does not return `Ok`
//! until that record is on disk. Drop the transaction (or call `abort`) and
//! nothing is durable.
//!
//! ```sh
//! cargo run -p pedradb-examples --example transactions
//! ```
//!
//! Next: `secondary_index` — the same TX primitive as a real layer.

use pedradb_core::{BatchOp, Db};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn parse_i64(bytes: &[u8]) -> i64 {
    std::str::from_utf8(bytes)
        .expect("utf-8 balance")
        .parse()
        .expect("i64")
}

fn transfer(db: &mut Db, from: &[u8], to: &[u8], amount: i64) -> pedradb_core::Result<bool> {
    let mut tx = db.begin();
    let src = parse_i64(&tx.get(from)?.expect("src account"));
    let dst = parse_i64(&tx.get(to)?.expect("dst account"));
    if src < amount {
        tx.abort();
        return Ok(false);
    }
    tx.put(from, (src - amount).to_string())?;
    tx.put(to, (dst + amount).to_string())?;
    let audit = format!(
        "audit/{}->{}:{amount}",
        String::from_utf8_lossy(from),
        String::from_utf8_lossy(to)
    );
    tx.put(audit.as_bytes(), b"ok")?;
    tx.commit()?;
    Ok(true)
}

fn run() -> pedradb_core::Result<()> {
    let dir = scratch("tx");
    let mut db = Db::open(&dir)?;

    db.put(b"acct/ada", b"100")?;
    db.put(b"acct/bob", b"40")?;

    assert!(transfer(&mut db, b"acct/ada", b"acct/bob", 25)?);
    assert_eq!(parse_i64(&db.get(b"acct/ada").expect("ada")), 75);
    assert_eq!(parse_i64(&db.get(b"acct/bob").expect("bob")), 65);

    // Insufficient funds: abort leaves balances untouched.
    assert!(!transfer(&mut db, b"acct/ada", b"acct/bob", 1_000)?);
    assert_eq!(parse_i64(&db.get(b"acct/ada").expect("ada")), 75);

    // Drop without commit is also abort.
    {
        let mut tx = db.begin();
        tx.put(b"acct/ada", b"0")?;
        tx.put(b"acct/bob", b"0")?;
        // drop
    }
    assert_eq!(parse_i64(&db.get(b"acct/ada").expect("ada")), 75);

    // Same atomicity without a live Transaction: one apply_batch, one WAL record.
    db.apply_batch([
        BatchOp::put(b"acct/ada", b"50"),
        BatchOp::put(b"acct/bob", b"90"),
        BatchOp::put(b"audit/batch", b"settled"),
    ])?;
    assert_eq!(parse_i64(&db.get(b"acct/ada").expect("ada")), 50);
    assert_eq!(db.get(b"audit/batch").as_deref(), Some(b"settled".as_ref()));

    println!("transactions: ada=50 bob=90 (atomic debit/credit + audit)");
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
        super::run().expect("transactions");
    }
}
