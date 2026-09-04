//! A tiny ledger: the kernel APIs chained into one program.
//!
//! Accounts, unique emails, atomic transfers with an ordered audit log,
//! a change-feed watcher, then drop the handle and reopen. Nothing here is
//! a new engine feature — it is `put` / `begin` / `scan` / `changes` composed.
//!
//! ```sh
//! cargo run -p pedradb-examples --example bank
//! ```
//!
//! Layers on the same kernel: `backup`, `rocksdb_dropin`.

use std::ops::Bound;

use pedradb_core::{prefix_exclusive_end, ChangeKind, Db, Result};

fn scratch(name: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("pedradb-ex-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn push_len(buf: &mut Vec<u8>, part: &[u8]) {
    let n = u32::try_from(part.len()).expect("component fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(part);
}

fn acct(name: &[u8]) -> Vec<u8> {
    let mut k = b"acct/".to_vec();
    push_len(&mut k, name);
    k
}

fn email_idx(email: &[u8]) -> Vec<u8> {
    let mut k = b"idx/".to_vec();
    push_len(&mut k, b"email");
    push_len(&mut k, email);
    k
}

fn xfer_key(seq: u64) -> Vec<u8> {
    format!("xfer/{seq:020}").into_bytes()
}

fn parse_i64(bytes: &[u8]) -> i64 {
    std::str::from_utf8(bytes)
        .expect("utf-8")
        .parse()
        .expect("i64")
}

fn balance(db: &Db, name: &[u8]) -> i64 {
    parse_i64(&db.get(&acct(name)).expect("account"))
}

fn open_account(db: &mut Db, name: &[u8], email: &[u8], cents: i64) -> Result<bool> {
    let pk = acct(name);
    let idx = email_idx(email);
    let mut tx = db.begin();
    if tx.get(&pk)?.is_some() {
        tx.abort();
        return Ok(false);
    }
    if tx.get(&idx)?.is_some() {
        tx.abort();
        return Ok(false);
    }
    tx.put(&pk, cents.to_string())?;
    tx.put(&idx, name)?;
    tx.commit()?;
    Ok(true)
}

fn transfer(db: &mut Db, from: &[u8], to: &[u8], cents: i64) -> Result<bool> {
    let src_k = acct(from);
    let dst_k = acct(to);
    let mut tx = db.begin();
    let src = parse_i64(&tx.get(&src_k)?.expect("src"));
    let dst = parse_i64(&tx.get(&dst_k)?.expect("dst"));
    if src < cents {
        tx.abort();
        return Ok(false);
    }
    tx.put(&src_k, (src - cents).to_string())?;
    tx.put(&dst_k, (dst + cents).to_string())?;
    // Counter lives under `meta/`, not `xfer/` — otherwise a prefix scan of
    // the ledger would also yield the counter (the footgun in ordered_scan).
    let next = tx
        .get(b"meta/next_xfer")?
        .map(|b| parse_i64(&b) + 1)
        .unwrap_or(1);
    tx.put(b"meta/next_xfer", next.to_string())?;
    tx.put(
        xfer_key(next as u64),
        format!(
            "{}>{}:{cents}",
            String::from_utf8_lossy(from),
            String::from_utf8_lossy(to)
        ),
    )?;
    tx.commit()?;
    Ok(true)
}

fn lookup_name(db: &Db, email: &[u8]) -> Option<Vec<u8>> {
    db.get(&email_idx(email)).map(|b| b.to_vec())
}

fn ledger(db: &Db) -> Vec<String> {
    let prefix = b"xfer/";
    let end = prefix_exclusive_end(prefix);
    let end_b = end.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
    db.scan(Bound::Included(prefix.as_ref()), end_b)
        .map(|kv| String::from_utf8_lossy(&kv.value).into_owned())
        .collect()
}

fn run() -> Result<()> {
    let dir = scratch("bank");
    let cursor;
    {
        let mut db = Db::open(&dir)?;
        cursor = db.last_sequence();

        assert!(open_account(&mut db, b"ada", b"ada@ex.com", 10_000)?);
        assert!(open_account(&mut db, b"bob", b"bob@ex.com", 4_000)?);
        // Unique email and unique name both refuse inside the same TX.
        assert!(!open_account(&mut db, b"cy", b"ada@ex.com", 1)?);
        assert!(!open_account(&mut db, b"ada", b"other@ex.com", 1)?);

        assert!(transfer(&mut db, b"ada", b"bob", 2_500)?);
        assert!(!transfer(&mut db, b"ada", b"bob", 1_000_000)?);
        assert_eq!(balance(&db, b"ada"), 7_500);
        assert_eq!(balance(&db, b"bob"), 6_500);

        let bob = lookup_name(&db, b"bob@ex.com").expect("email idx");
        assert_eq!(bob, b"bob");

        let log = ledger(&db);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0], "ada>bob:2500");

        let feed = db.changes(cursor, db.last_sequence())?;
        assert!(feed.iter().any(|e| e.kind == ChangeKind::Put));

        println!(
            "bank: ada=7500 bob=6500  ledger={}  feed={} events — dropping handle",
            log.len(),
            feed.len()
        );
        // No close: same contract as crash_reopen.
    }

    let db = Db::open(&dir)?;
    assert_eq!(balance(&db, b"ada"), 7_500);
    assert_eq!(balance(&db, b"bob"), 6_500);
    assert_eq!(
        lookup_name(&db, b"ada@ex.com").as_deref(),
        Some(b"ada".as_ref())
    );
    assert_eq!(ledger(&db), ["ada>bob:2500"]);
    println!(
        "bank: reopen ok — balances and ledger survived (seq={})",
        db.last_sequence()
    );
    db.close()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn main() -> Result<()> {
    run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::run().expect("bank");
    }
}
