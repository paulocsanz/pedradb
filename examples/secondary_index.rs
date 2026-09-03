//! Secondary-index layer sketch on PedraDB multi-key TX (RFC-0009 P0.3):
//! a row and its index entry land (or abort) in one atomic, durable commit.
//!
//! Run from the repo root:
//!   cargo run --example secondary_index -p pedradb-core

use pedradb_core::{Db, OpenOptions, Result};

/// Toy user row + email index using byte keys only.
fn upsert_user(db: &mut Db, id: &[u8], email: &[u8], payload: &[u8]) -> Result<()> {
    let mut pk = b"u/".to_vec();
    pk.extend_from_slice(id);
    let mut idx = b"idx/email/".to_vec();
    idx.extend_from_slice(email);

    let mut tx = db.begin();
    tx.put(&pk, payload)?;
    tx.put(&idx, id)?;
    tx.commit().map(|_| ())
}

fn lookup_id_by_email(db: &Db, email: &[u8]) -> Option<Vec<u8>> {
    let mut idx = b"idx/email/".to_vec();
    idx.extend_from_slice(email);
    db.get(&idx).map(|b| b.to_vec())
}

fn main() -> Result<()> {
    let dir = std::env::temp_dir().join("pedradb-index-example");
    let _ = std::fs::remove_dir_all(&dir);
    let mut db = Db::open_with(
        &dir,
        OpenOptions {
            wal_full_fsync: false,
            history: Default::default(),
            wal_recovery: Default::default(),
            sync: true,
            auto_flush_bytes: Some(1024 * 1024),
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
            sst_payload_budget_bytes: None,
        },
    )?;

    upsert_user(&mut db, b"42", b"ada@ex.com", br#"{"name":"ada"}"#)?;
    let id = lookup_id_by_email(&db, b"ada@ex.com").expect("index hit");
    assert_eq!(id, b"42");

    // Abort path: neither key should land
    {
        let mut tx = db.begin();
        tx.put(b"u/99", b"nope")?;
        tx.put(b"idx/email/bad@ex.com", b"99")?;
        tx.abort();
    }
    assert!(db.get(b"u/99").is_none());
    assert!(lookup_id_by_email(&db, b"bad@ex.com").is_none());

    println!(
        "secondary index example ok (id={})",
        String::from_utf8_lossy(&id)
    );
    db.close()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
