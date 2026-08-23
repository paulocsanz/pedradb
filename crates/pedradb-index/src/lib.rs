//! Secondary-index canary (RFC-0020 P1.1 / W3 `index-tx-crash`).
//!
//! One multi-key TX writes primary row + ≥2 secondary index keys. Crash mid-TX
//! must leave **all or none** (never half-index).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use pedradb_core::{Db, OpenOptions, Result, StdEnv};
use std::path::Path;

/// Primary key prefix: `row/` + length-prefixed id (F89).
pub const ROW_PREFIX: &[u8] = b"row/";
/// Secondary index namespace (F65).
pub const IDX_PREFIX: &[u8] = b"idx/";

fn push_len_pref(buf: &mut Vec<u8>, part: &[u8]) {
    let n = u32::try_from(part.len()).expect("component len fits u32");
    buf.extend_from_slice(&n.to_be_bytes());
    buf.extend_from_slice(part);
}

/// Build primary key for row id.
///
/// F89: raw `row/ || id` made `row/a` a byte-prefix of `row/ab`, so any
/// half-open prefix scan of one id leaked sibling rows. Length-prefix the id
/// (same framing as [`idx_key`]).
#[must_use]
pub fn row_key(id: impl AsRef<[u8]>) -> Vec<u8> {
    let mut k = ROW_PREFIX.to_vec();
    push_len_pref(&mut k, id.as_ref());
    k
}

/// Build secondary index key for `(field, value)` → row id (F65).
///
/// - Slash join: `idx/name/a/b` equaled both `(name, a/b)` and `(name/a, b)`.
/// - `0x00` join: `(a, b\\0c)` equaled `(a\\0b, c)`.
/// Length-prefix each component after `idx/`.
#[must_use]
pub fn idx_key(field: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Vec<u8> {
    let mut k = IDX_PREFIX.to_vec();
    push_len_pref(&mut k, field.as_ref());
    push_len_pref(&mut k, value.as_ref());
    k
}

/// Insert row + two secondary indexes in **one** TX commit.
///
/// # Errors
/// WAL / TX I/O.
pub fn put_row_with_indexes(
    db: &mut Db<StdEnv>,
    id: impl AsRef<[u8]>,
    payload: impl AsRef<[u8]>,
    name: impl AsRef<[u8]>,
    email: impl AsRef<[u8]>,
) -> Result<()> {
    let id = id.as_ref();
    let mut tx = db.begin();
    tx.put(row_key(id), payload.as_ref())?;
    tx.put(idx_key(b"name", name.as_ref()), id)?;
    tx.put(idx_key(b"email", email.as_ref()), id)?;
    tx.commit()?;
    Ok(())
}

/// Whether all three keys for a row are present (consistent index).
#[must_use]
pub fn row_fully_indexed(
    db: &Db<StdEnv>,
    id: impl AsRef<[u8]>,
    name: impl AsRef<[u8]>,
    email: impl AsRef<[u8]>,
) -> bool {
    let id = id.as_ref();
    let has_row = db.get(&row_key(id)).is_some();
    let has_name = db.get(&idx_key(b"name", name.as_ref())).as_deref() == Some(id);
    let has_email = db.get(&idx_key(b"email", email.as_ref())).as_deref() == Some(id);
    has_row && has_name && has_email
}

/// Half-index: some but not all of the three keys exist.
#[must_use]
pub fn row_half_indexed(
    db: &Db<StdEnv>,
    id: impl AsRef<[u8]>,
    name: impl AsRef<[u8]>,
    email: impl AsRef<[u8]>,
) -> bool {
    let id = id.as_ref();
    let bits = [
        db.get(&row_key(id)).is_some(),
        db.get(&idx_key(b"name", name.as_ref())).as_deref() == Some(id),
        db.get(&idx_key(b"email", email.as_ref())).as_deref() == Some(id),
    ];
    let n = bits.iter().filter(|b| **b).count();
    n > 0 && n < 3
}

/// Workload report for W3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadReport {
    /// Workload name.
    pub name: &'static str,
    /// Trials.
    pub trials: u64,
    /// Must be 0.
    pub silent_wrong: u64,
    /// Detail.
    pub detail: String,
}

/// W3: commit succeeds → reopen sees full index; uncommitted staging never durable.
///
/// # Errors
/// I/O.
pub fn workload_index_tx_crash(dir: impl AsRef<Path>) -> Result<WorkloadReport> {
    let dir = dir.as_ref();
    let opts = OpenOptions {
        wal_full_fsync: false,
        history: Default::default(),
        wal_recovery: Default::default(),
        sync: true,
        auto_flush_bytes: None,
        auto_compact_sst_count: None,
        auto_compact_sst_bytes: None,
        exclusive: true,
        large_value_threshold: None,
    };
    let mut silent_wrong = 0u64;
    let mut trials = 0u64;

    // Trial A: successful multi-key commit, process exit, reopen → full index.
    {
        trials += 1;
        let mut db = Db::open_with(dir, opts)?;
        put_row_with_indexes(&mut db, b"42", br#"{"n":"ada"}"#, b"ada", b"a@x")?;
        drop(db);
        let db = Db::open_with(dir, opts)?;
        if !row_fully_indexed(&db, b"42", b"ada", b"a@x") {
            silent_wrong += 1;
        }
        if row_half_indexed(&db, b"42", b"ada", b"a@x") {
            silent_wrong += 1;
        }
        drop(db);
    }

    // Trial B: begin + stage without commit → reopen must not see half-index.
    {
        trials += 1;
        let dir_b = dir.join("uncommitted");
        let _ = std::fs::create_dir_all(&dir_b);
        {
            let mut db = Db::open_with(&dir_b, opts)?;
            let mut tx = db.begin();
            tx.put(row_key(b"99"), b"partial")?;
            tx.put(idx_key(b"name", b"bob"), b"99")?;
            // no commit — drop TX
            drop(tx);
            drop(db);
        }
        let db = Db::open_with(&dir_b, opts)?;
        if row_half_indexed(&db, b"99", b"bob", b"b@x") {
            silent_wrong += 1;
        }
        if db.get(&row_key(b"99")).is_some() {
            silent_wrong += 1;
        }
        if db.get(&idx_key(b"name", b"bob")).is_some() {
            silent_wrong += 1;
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    // Trial C: many successful rows — no half keys after reopen.
    {
        trials += 1;
        let dir_c = dir.join("multi");
        let _ = std::fs::create_dir_all(&dir_c);
        {
            let mut db = Db::open_with(&dir_c, opts)?;
            for i in 0..20u8 {
                let id = [b'i', i];
                let name = [b'n', i];
                let email = [b'e', i];
                put_row_with_indexes(&mut db, id, b"p", name, email)?;
            }
            drop(db);
        }
        let db = Db::open_with(&dir_c, opts)?;
        for i in 0..20u8 {
            let id = [b'i', i];
            let name = [b'n', i];
            let email = [b'e', i];
            if !row_fully_indexed(&db, id, name, email) || row_half_indexed(&db, id, name, email) {
                silent_wrong += 1;
            }
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir_c);
    }

    Ok(WorkloadReport {
        name: "index-tx-crash",
        trials,
        silent_wrong,
        detail: format!("silent_wrong={silent_wrong}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let i = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pedra-index-{n}-{i}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn row_key_id_is_not_prefix_of_sibling_id() {
        // F89 AS-IS: `row/` || id → `row/a` is a prefix of `row/ab`.
        let a = row_key(b"a");
        let ab = row_key(b"ab");
        assert!(
            !ab.starts_with(&a),
            "row_key(a) must not be a byte-prefix of row_key(ab): {a:?} vs {ab:?}"
        );
        assert_ne!(a, ab);
        // NUL / slash in ids stay distinct point keys.
        assert_ne!(row_key(b"a/b"), row_key(b"a"));
        assert_ne!(row_key(b"a\0b"), row_key(b"a"));
        let dir = temp();
        let mut db = Db::open(&dir).unwrap();
        put_row_with_indexes(&mut db, b"a", b"va", b"n1", b"e1").unwrap();
        put_row_with_indexes(&mut db, b"ab", b"vab", b"n2", b"e2").unwrap();
        assert_eq!(db.get(&row_key(b"a")).as_deref(), Some(b"va".as_ref()));
        assert_eq!(db.get(&row_key(b"ab")).as_deref(), Some(b"vab".as_ref()));
        // Prefix scan of exact row_key("a") must not include "ab".
        let end = pedradb_core::prefix_exclusive_end(&a);
        let hits: Vec<_> = db
            .range_limited(
                std::ops::Bound::Included(a.as_slice()),
                match end.as_deref() {
                    Some(e) => std::ops::Bound::Excluded(e),
                    None => std::ops::Bound::Unbounded,
                },
                None,
            )
            .into_iter()
            .map(|(k, _)| k.to_vec())
            .collect();
        assert_eq!(
            hits,
            vec![a.clone()],
            "prefix scan leaked siblings: {hits:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn idx_key_is_injective_when_components_contain_slash() {
        assert_ne!(
            idx_key(b"name", b"a/b"),
            idx_key(b"name/a", b"b"),
            "idx/field/value slash join collided"
        );
        assert_ne!(idx_key(b"name", b"a"), idx_key(b"nam", b"e/a"));
        // 0x00 join also collides without length-prefix.
        assert_ne!(
            idx_key(b"a", b"b\x00c"),
            idx_key(b"a\x00b", b"c"),
            "idx field\\0value NUL join collided"
        );
        let dir = temp();
        let mut db = Db::open(&dir).unwrap();
        put_row_with_indexes(&mut db, b"1", b"body", b"a/b", b"e1").unwrap();
        put_row_with_indexes(&mut db, b"2", b"body2", b"x", b"e2").unwrap();
        assert_eq!(
            db.get(&idx_key(b"name", b"a/b")).as_deref(),
            Some(b"1".as_ref()),
            "name a/b must stay id 1"
        );
        // A second row whose slash-joined key used to overwrite the first.
        db.put(&idx_key(b"name/a", b"b"), b"2").unwrap();
        assert_eq!(
            db.get(&idx_key(b"name", b"a/b")).as_deref(),
            Some(b"1".as_ref()),
            "writing name/a + b must not clobber name + a/b"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_and_lookup_indexes() {
        let dir = temp();
        let mut db = Db::open(&dir).unwrap();
        put_row_with_indexes(&mut db, b"1", b"body", b"alice", b"a@e").unwrap();
        assert!(row_fully_indexed(&db, b"1", b"alice", b"a@e"));
        assert!(!row_half_indexed(&db, b"1", b"alice", b"a@e"));
        assert_eq!(
            db.get(&idx_key(b"name", b"alice")).as_deref(),
            Some(b"1".as_ref())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_tx_crash_silent_wrong_zero() {
        let dir = temp();
        let r = workload_index_tx_crash(&dir).unwrap();
        assert_eq!(r.silent_wrong, 0, "{r:?}");
        assert_eq!(r.name, "index-tx-crash");
        let _ = fs::remove_dir_all(&dir);
    }
}
