//! A secondary index is just more keys in the same transaction.
//!
//! PedraDB does not implement indexes. You do: primary row + `idx/email/…`
//! land (or abort) together. Crash mid-commit cannot leave a half-index —
//! recovery either has the whole WAL record or skips the truncated tail.
//!
//! Keys are length-prefixed so `idx/email/a` is not a byte-prefix of
//! `idx/email/ab` (the classic "slash join" footgun).
//!
//! ```sh
//! cargo run -p pedradb-examples --example secondary_index
//! ```
//!
//! Next: `ordered_scan` — list every row under a prefix, in order.

use pedradb_core::{Db, Result};

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

fn row_key(id: &[u8]) -> Vec<u8> {
    let mut k = b"row/".to_vec();
    push_len(&mut k, id);
    k
}

fn idx_email(email: &[u8]) -> Vec<u8> {
    let mut k = b"idx/".to_vec();
    push_len(&mut k, b"email");
    push_len(&mut k, email);
    k
}

/// Insert or update a user. Email uniqueness is checked inside the TX.
fn upsert_user(db: &mut Db, id: &[u8], email: &[u8], payload: &[u8]) -> Result<bool> {
    let pk = row_key(id);
    let idx = idx_email(email);
    let mut tx = db.begin();

    if let Some(owner) = tx.get(&idx)? {
        if owner.as_ref() != id {
            tx.abort();
            return Ok(false); // email taken by someone else
        }
    }

    // Moving email: drop the previous index entry in the same commit.
    if let Some(old) = tx.get(&pk)? {
        if let Some(old_email) = old.split(|&b| b == 0).next() {
            if old_email != email {
                tx.delete(idx_email(old_email))?;
            }
        }
    }

    let mut value = email.to_vec();
    value.push(0);
    value.extend_from_slice(payload);
    tx.put(&pk, &value)?;
    tx.put(&idx, id)?;
    tx.commit()?;
    Ok(true)
}

fn lookup_id_by_email(db: &Db, email: &[u8]) -> Option<Vec<u8>> {
    db.get(&idx_email(email)).map(|b| b.to_vec())
}

fn run() -> Result<()> {
    let dir = scratch("index");
    let mut db = Db::open(&dir)?;

    assert!(upsert_user(
        &mut db,
        b"42",
        b"ada@ex.com",
        br#"{"name":"ada"}"#
    )?);
    let id = lookup_id_by_email(&db, b"ada@ex.com").expect("index hit");
    assert_eq!(id, b"42");

    // Change email: old index gone, new index present, row updated — one commit.
    assert!(upsert_user(
        &mut db,
        b"42",
        b"ada@cs.ex.com",
        br#"{"name":"ada"}"#
    )?);
    assert!(lookup_id_by_email(&db, b"ada@ex.com").is_none());
    assert_eq!(
        lookup_id_by_email(&db, b"ada@cs.ex.com").as_deref(),
        Some(b"42".as_ref())
    );

    // Unique email: bob cannot steal ada's address.
    assert!(!upsert_user(&mut db, b"99", b"ada@cs.ex.com", b"nope")?);
    assert!(db.get(&row_key(b"99")).is_none());

    // Abort: neither the row nor the index lands.
    {
        let mut tx = db.begin();
        tx.put(row_key(b"7"), b"ghost")?;
        tx.put(idx_email(b"ghost@ex.com"), b"7")?;
        tx.abort();
    }
    assert!(db.get(&row_key(b"7")).is_none());
    assert!(lookup_id_by_email(&db, b"ghost@ex.com").is_none());

    println!("secondary_index: ada@cs.ex.com → 42 (unique, atomic, abort-safe)");
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
        super::run().expect("secondary_index");
    }
}
