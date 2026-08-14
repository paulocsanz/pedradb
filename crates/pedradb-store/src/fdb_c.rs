//! Thin C ABI for FDB-shaped plug tests (RFC-0023 P2.2).
//!
//! **Not** full FoundationDB C bindings / fdbcli. Opaque handles over an owned
//! in-process [`StoreCluster`] + [`Transaction`](crate::client::Transaction).
//!
//! Enable with `--features c-api`. Header: `include/montanha_fdb.h`.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::ptr;

use crate::client::Transaction;
use crate::fdb_compat::FdbError;
use crate::{StoreCluster, StoreError};

/// Success.
pub const MONTAHA_FDB_OK: c_int = 0;
/// Conflict / not_committed.
pub const MONTAHA_FDB_NOT_COMMITTED: c_int = 1;
/// Snapshot too old.
pub const MONTAHA_FDB_TOO_OLD: c_int = 2;
/// Value / TX limit.
pub const MONTAHA_FDB_LIMIT: c_int = 3;
/// Unavailable / not leader / not committed majority.
pub const MONTAHA_FDB_UNAVAILABLE: c_int = 4;
/// Other / invalid args.
pub const MONTAHA_FDB_ERROR: c_int = 5;

/// Owned cluster for C consumers.
pub struct MontanhaFdbDatabase {
    cluster: StoreCluster,
}

/// Pending native transaction.
pub struct MontanhaFdbTransaction {
    inner: Transaction,
}

fn map_err(e: &StoreError) -> c_int {
    match FdbError::from_store(e) {
        FdbError::NotCommitted => MONTAHA_FDB_NOT_COMMITTED,
        FdbError::TransactionTooOld => MONTAHA_FDB_TOO_OLD,
        FdbError::Limit => MONTAHA_FDB_LIMIT,
        FdbError::Unavailable(_) => MONTAHA_FDB_UNAVAILABLE,
        FdbError::Other(_) => MONTAHA_FDB_ERROR,
    }
}

/// Open an in-process  cluster under `path` (`n_nodes` peers, `n_ranges` splits).
///
/// On success returns a heap handle; free with [`montanha_fdb_database_destroy`].
///
/// # Safety
/// `path` must be a valid NUL-terminated C string (or null → temp-style caller path required).
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_database_create(
    path: *const c_char,
    n_nodes: u64,
    n_ranges: u64,
) -> *mut MontanhaFdbDatabase {
    if path.is_null() || n_nodes == 0 || n_ranges == 0 {
        return ptr::null_mut();
    }
    let cstr = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let p = PathBuf::from(cstr);
    match StoreCluster::open(&p, n_nodes, n_ranges) {
        Ok(mut cluster) => {
            if cluster.elect_all(120).is_err() {
                return ptr::null_mut();
            }
            Box::into_raw(Box::new(MontanhaFdbDatabase { cluster }))
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Destroy a database handle.
///
/// # Safety
/// `db` must be null or a pointer from [`montanha_fdb_database_create`].
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_database_destroy(db: *mut MontanhaFdbDatabase) {
    if !db.is_null() {
        drop(Box::from_raw(db));
    }
}

/// Create a snapshot transaction (caller frees with destroy or after commit).
///
/// # Safety
/// `db` must be a live database handle.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_create(
    db: *mut MontanhaFdbDatabase,
) -> *mut MontanhaFdbTransaction {
    if db.is_null() {
        return ptr::null_mut();
    }
    let d = &mut *db;
    let inner = d.cluster.begin();
    Box::into_raw(Box::new(MontanhaFdbTransaction { inner }))
}

/// Destroy a transaction that was not committed (or after failed commit).
///
/// # Safety
/// `tr` null or from create / failed commit.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_destroy(tr: *mut MontanhaFdbTransaction) {
    if !tr.is_null() {
        drop(Box::from_raw(tr));
    }
}

/// Stage a set. Returns error code.
///
/// # Safety
/// Valid handles; `key`/`value` valid for `key_len`/`value_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_set(
    tr: *mut MontanhaFdbTransaction,
    key: *const u8,
    key_len: usize,
    value: *const u8,
    value_len: usize,
) -> c_int {
    if tr.is_null() || key.is_null() || (value.is_null() && value_len > 0) {
        return MONTAHA_FDB_ERROR;
    }
    let t = &mut *tr;
    let k = std::slice::from_raw_parts(key, key_len);
    let v = if value_len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(value, value_len)
    };
    match t.inner.set(k, v) {
        Ok(()) => MONTAHA_FDB_OK,
        Err(e) => map_err(&e),
    }
}

/// Snapshot get. On success writes length to `out_len` and pointer into `out_ptr`
/// (free with [`montanha_fdb_free`]`(p, len)`). Missing key → OK, null, len 0.
///
/// # Safety
/// Valid handles and output pointers.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_get(
    db: *mut MontanhaFdbDatabase,
    tr: *mut MontanhaFdbTransaction,
    key: *const u8,
    key_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    if db.is_null() || tr.is_null() || key.is_null() || out_ptr.is_null() || out_len.is_null() {
        return MONTAHA_FDB_ERROR;
    }
    let d = &mut *db;
    let t = &mut *tr;
    let k = std::slice::from_raw_parts(key, key_len);
    match t.inner.get(&d.cluster, k) {
        Ok(None) => {
            *out_ptr = ptr::null_mut();
            *out_len = 0;
            MONTAHA_FDB_OK
        }
        Ok(Some(v)) => {
            let n = v.len();
            if n == 0 {
                *out_ptr = ptr::null_mut();
                *out_len = 0;
                return MONTAHA_FDB_OK;
            }
            let mut boxed = v.into_boxed_slice();
            let p = boxed.as_mut_ptr();
            std::mem::forget(boxed);
            *out_ptr = p;
            *out_len = n;
            MONTAHA_FDB_OK
        }
        Err(e) => map_err(&e),
    }
}

/// Commit transaction. Consumes `tr` on all paths (do not destroy again).
///
/// # Safety
/// Live `db` and `tr`.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_commit(
    db: *mut MontanhaFdbDatabase,
    tr: *mut MontanhaFdbTransaction,
) -> c_int {
    if db.is_null() || tr.is_null() {
        return MONTAHA_FDB_ERROR;
    }
    let d = &mut *db;
    let t = Box::from_raw(tr);
    match t.inner.commit(&mut d.cluster) {
        Ok(_) => MONTAHA_FDB_OK,
        Err(e) => map_err(&e),
    }
}

/// Free a buffer from [`montanha_fdb_transaction_get`] (`len` must match `out_len`).
///
/// # Safety
/// `p` null or from get with the same `len`.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_free(p: *mut u8, len: usize) {
    if p.is_null() || len == 0 {
        return;
    }
    drop(Box::from_raw(std::slice::from_raw_parts_mut(p, len)));
}

// ── Rust-side smoke (no full C toolchain required in unit tests) ───────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn c_api_open_set_get_commit() {
        let dir = std::env::temp_dir().join(format!(
            "montanha-fdb-c-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null());
            let tr = montanha_fdb_transaction_create(db);
            assert!(!tr.is_null());
            let key = b"c/k";
            let val = b"v1";
            assert_eq!(
                montanha_fdb_transaction_set(tr, key.as_ptr(), key.len(), val.as_ptr(), val.len()),
                MONTAHA_FDB_OK
            );
            assert_eq!(montanha_fdb_transaction_commit(db, tr), MONTAHA_FDB_OK);

            let tr2 = montanha_fdb_transaction_create(db);
            let mut out: *mut u8 = ptr::null_mut();
            let mut len: usize = 0;
            assert_eq!(
                montanha_fdb_transaction_get(
                    db,
                    tr2,
                    key.as_ptr(),
                    key.len(),
                    &mut out,
                    &mut len
                ),
                MONTAHA_FDB_OK
            );
            assert_eq!(len, 2);
            assert_eq!(std::slice::from_raw_parts(out, len), b"v1");
            montanha_fdb_free(out, len);
            montanha_fdb_transaction_destroy(tr2);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn c_api_conflict_code() {
        let dir = std::env::temp_dir().join(format!(
            "montanha-fdb-c-cf-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            let t1 = montanha_fdb_transaction_create(db);
            let t2 = montanha_fdb_transaction_create(db);
            let k = b"x";
            montanha_fdb_transaction_set(t1, k.as_ptr(), 1, b"1".as_ptr(), 1);
            montanha_fdb_transaction_set(t2, k.as_ptr(), 1, b"2".as_ptr(), 1);
            assert_eq!(montanha_fdb_transaction_commit(db, t1), MONTAHA_FDB_OK);
            assert_eq!(
                montanha_fdb_transaction_commit(db, t2),
                MONTAHA_FDB_NOT_COMMITTED
            );
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
