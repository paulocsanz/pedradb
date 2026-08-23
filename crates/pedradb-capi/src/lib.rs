//! Thin C ABI for FDB-shaped plug tests (RFC-0023 P2.2).
//!
//! **Not** FoundationDB / fdbcli / a supported product ABI. Opaque **handles**
//! (slot + generation packed into the pointer) over [`StoreCluster`] +
//! [`Transaction`]. Double-destroy and use-after-destroy return an error /
//! NULL — they are not `Box::from_raw` UB. Handle tables are **thread-local**
//! (`StoreCluster` is `!Send`). A handle used on another thread yields ERROR,
//! not a data race.
//!
//! Header: `include/montanha_fdb.h`. Build: `cargo build -p pedradb-capi`.
//!
//! Remaining `unsafe` is marshalling only: NUL-terminated `path`, and
//! `key`/`value` readable for `*_len` bytes. Invariants: crate `SAFETY.md`.

#![warn(missing_docs)]

mod handles;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::ptr;

use pedradb_store::client::Transaction;
use pedradb_store::fdb_compat::FdbError;
use pedradb_store::{StoreCluster, StoreError};

use handles::{pack, unpack, Handle, Table, KIND_DB, KIND_TX};

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
/// Other / invalid args / stale handle.
pub const MONTAHA_FDB_ERROR: c_int = 5;

/// Opaque C database handle (packed id, not a heap pointer).
#[repr(C)]
pub struct MontanhaFdbDatabase {
    _opaque: [u8; 0],
}

/// Opaque C transaction handle (packed id, not a heap pointer).
#[repr(C)]
pub struct MontanhaFdbTransaction {
    _opaque: [u8; 0],
}

struct TxSlot {
    db: Handle,
    inner: Transaction,
}

struct CapiState {
    dbs: Table<StoreCluster>,
    txs: Table<TxSlot>,
    /// Address → length of a `Box<[u8]>` given to C via `into_raw`.
    /// Free reconstructs with `from_raw` only for keys present here.
    bufs: HashMap<usize, usize>,
}

impl CapiState {
    fn new() -> Self {
        Self {
            dbs: Table::new(),
            txs: Table::new(),
            bufs: HashMap::new(),
        }
    }
}

thread_local! {
    static STATE: RefCell<CapiState> = RefCell::new(CapiState::new());
}

fn with_state<R>(f: impl FnOnce(&mut CapiState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
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

fn db_ptr(h: Handle) -> *mut MontanhaFdbDatabase {
    pack(KIND_DB, h) as *mut MontanhaFdbDatabase
}

fn tx_ptr(h: Handle) -> *mut MontanhaFdbTransaction {
    pack(KIND_TX, h) as *mut MontanhaFdbTransaction
}

fn db_handle(p: *mut MontanhaFdbDatabase) -> Option<Handle> {
    unpack(KIND_DB, p as usize as u64)
}

fn tx_handle(p: *mut MontanhaFdbTransaction) -> Option<Handle> {
    unpack(KIND_TX, p as usize as u64)
}

/// Open an in-process cluster under `path` (`n_nodes` peers, `n_ranges` splits).
///
/// On success returns a handle; free with [`montanha_fdb_database_destroy`].
///
/// # Safety
/// `path` must be a valid NUL-terminated C string (or null).
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_database_create(
    path: *const c_char,
    n_nodes: u64,
    n_ranges: u64,
) -> *mut MontanhaFdbDatabase {
    if path.is_null() || n_nodes == 0 || n_ranges == 0 {
        return ptr::null_mut();
    }
    // SAFETY: caller contract — `path` is NUL-terminated and readable.
    let cstr = unsafe { CStr::from_ptr(path) };
    let s = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let p = PathBuf::from(s);
    match StoreCluster::open(&p, n_nodes, n_ranges) {
        Ok(mut cluster) => {
            if cluster.elect_all(120).is_err() {
                return ptr::null_mut();
            }
            let h = with_state(|st| st.dbs.insert(cluster));
            db_ptr(h)
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Destroy a database handle. Null or stale is a no-op (not UB).
///
/// Live transactions on this database are dropped.
///
/// # Safety
/// `db` is null or a pointer returned by [`montanha_fdb_database_create`].
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_database_destroy(db: *mut MontanhaFdbDatabase) {
    let Some(h) = db_handle(db) else {
        return;
    };
    with_state(|st| {
        if st.dbs.remove(h).is_some() {
            st.txs.drain_if(|t| t.db == h);
        }
    });
}

/// Create a snapshot transaction (caller frees with destroy or after commit).
///
/// # Safety
/// `db` is a live database handle.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_create(
    db: *mut MontanhaFdbDatabase,
) -> *mut MontanhaFdbTransaction {
    let Some(dh) = db_handle(db) else {
        return ptr::null_mut();
    };
    with_state(|st| {
        let Some(cluster) = st.dbs.get(dh) else {
            return ptr::null_mut();
        };
        let inner = cluster.begin();
        let h = st.txs.insert(TxSlot { db: dh, inner });
        tx_ptr(h)
    })
}

/// Destroy a transaction that was not committed (or after failed commit).
/// Null or stale is a no-op. After [`montanha_fdb_transaction_commit`] the
/// handle is already consumed — destroy is a no-op, not a double-free.
///
/// # Safety
/// `tr` is null or a pointer from create / a consumed commit handle.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_destroy(tr: *mut MontanhaFdbTransaction) {
    let Some(h) = tx_handle(tr) else {
        return;
    };
    with_state(|st| {
        let _ = st.txs.remove(h);
    });
}

/// Stage a set. Returns error code.
///
/// # Safety
/// `key` (and `value` when `value_len > 0`) must be readable for the given
/// lengths. `tr` is a handle from create.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_set(
    tr: *mut MontanhaFdbTransaction,
    key: *const u8,
    key_len: usize,
    value: *const u8,
    value_len: usize,
) -> c_int {
    if key.is_null() || (value.is_null() && value_len > 0) {
        return MONTAHA_FDB_ERROR;
    }
    let Some(h) = tx_handle(tr) else {
        return MONTAHA_FDB_ERROR;
    };
    // SAFETY: caller contract — `key`/`value` live for `*_len` bytes.
    let k = unsafe { std::slice::from_raw_parts(key, key_len) };
    let v = if value_len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(value, value_len) }
    };
    with_state(|st| {
        let Some(t) = st.txs.get_mut(h) else {
            return MONTAHA_FDB_ERROR;
        };
        match t.inner.set(k, v) {
            Ok(()) => MONTAHA_FDB_OK,
            Err(e) => map_err(&e),
        }
    })
}

/// Snapshot get. On success writes length to `out_len` and pointer into
/// `out_ptr` (free with [`montanha_fdb_free`]). Missing / empty → OK, null, 0.
/// Error also writes null/0 so the caller never reads uninitialized outputs.
///
/// # Safety
/// `key` readable for `key_len`. `out_ptr` / `out_len` live. Handles live
/// and belong together (`tr` was created from `db`).
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_get(
    db: *mut MontanhaFdbDatabase,
    tr: *mut MontanhaFdbTransaction,
    key: *const u8,
    key_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> c_int {
    if key.is_null() || out_ptr.is_null() || out_len.is_null() {
        return MONTAHA_FDB_ERROR;
    }
    let Some(dh) = db_handle(db) else {
        return MONTAHA_FDB_ERROR;
    };
    let Some(th) = tx_handle(tr) else {
        return MONTAHA_FDB_ERROR;
    };
    // SAFETY: caller contract — `key` live for `key_len`.
    let k = unsafe { std::slice::from_raw_parts(key, key_len) };
    with_state(|st| {
        if st.txs.get(th).map(|t| t.db) != Some(dh) {
            // SAFETY: `out_*` checked non-null above.
            unsafe {
                *out_ptr = ptr::null_mut();
                *out_len = 0;
            }
            return MONTAHA_FDB_ERROR;
        }
        let got = {
            let Some(cluster) = st.dbs.get(dh) else {
                unsafe {
                    *out_ptr = ptr::null_mut();
                    *out_len = 0;
                }
                return MONTAHA_FDB_ERROR;
            };
            let cluster = cluster as *const StoreCluster;
            let t = st.txs.get_mut(th).expect("checked");
            // SAFETY: `cluster` is `st.dbs[dh]`, `t` is `st.txs[th]` — disjoint
            // slots, both live for this `with_state` call.
            t.inner.get(unsafe { &*cluster }, k)
        };
        match got {
            Ok(None) => {
                unsafe {
                    *out_ptr = ptr::null_mut();
                    *out_len = 0;
                }
                MONTAHA_FDB_OK
            }
            Ok(Some(v)) if v.is_empty() => {
                unsafe {
                    *out_ptr = ptr::null_mut();
                    *out_len = 0;
                }
                MONTAHA_FDB_OK
            }
            Ok(Some(v)) => {
                let boxed = v.into_boxed_slice();
                let n = boxed.len();
                // `into_raw` (not `as_mut_ptr` then move the Box): the
                // pointer handed to C stays the owner. Moving the Box into
                // a HashMap after `as_mut_ptr` invalidates that pointer
                // (Stacked Borrows Unique retag — F210 / Miri).
                let p = Box::into_raw(boxed).cast::<u8>();
                st.bufs.insert(p as usize, n);
                unsafe {
                    *out_ptr = p;
                    *out_len = n;
                }
                MONTAHA_FDB_OK
            }
            Err(e) => {
                unsafe {
                    *out_ptr = ptr::null_mut();
                    *out_len = 0;
                }
                map_err(&e)
            }
        }
    })
}

/// Commit transaction. Consumes `tr` on all paths that find a live handle
/// (destroy afterwards is a no-op).
///
/// # Safety
/// Live `db` and `tr` that belong together.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_transaction_commit(
    db: *mut MontanhaFdbDatabase,
    tr: *mut MontanhaFdbTransaction,
) -> c_int {
    let Some(dh) = db_handle(db) else {
        return MONTAHA_FDB_ERROR;
    };
    let Some(th) = tx_handle(tr) else {
        return MONTAHA_FDB_ERROR;
    };
    with_state(|st| {
        match st.txs.get(th) {
            Some(t) if t.db == dh => {}
            _ => return MONTAHA_FDB_ERROR,
        }
        if st.dbs.get(dh).is_none() {
            return MONTAHA_FDB_ERROR;
        }
        let slot = st.txs.remove(th).expect("checked");
        let cluster = st.dbs.get_mut(dh).expect("checked");
        match slot.inner.commit(cluster) {
            Ok(_) => MONTAHA_FDB_OK,
            Err(e) => map_err(&e),
        }
    })
}

/// Free a buffer from [`montanha_fdb_transaction_get`]. Null, unknown, or
/// already-freed pointers are no-ops (not allocator UB). `len` is ignored
/// for ownership; the table knows the real allocation.
///
/// # Safety
/// `p` is null or a pointer previously written to `out_ptr` (or garbage,
/// which is ignored).
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_free(p: *mut u8, len: usize) {
    let _ = len;
    if p.is_null() {
        return;
    }
    with_state(|st| {
        if let Some(n) = st.bufs.remove(&(p as usize)) {
            // SAFETY: `p` was `Box::into_raw` of a `Box<[u8]>` of length `n`
            // in `transaction_get`. Unknown / double-free pointers never
            // reach `from_raw` (they miss the table).
            let slice = ptr::slice_from_raw_parts_mut(p, n);
            drop(unsafe { Box::from_raw(slice) });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn temp_path() -> (std::path::PathBuf, CString) {
        let dir = std::env::temp_dir().join(format!(
            "montanha-fdb-c-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        (dir, path)
    }

    #[test]
    fn c_api_open_set_get_commit() {
        let (dir, path) = temp_path();
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
                montanha_fdb_transaction_get(db, tr2, key.as_ptr(), key.len(), &mut out, &mut len),
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
        let (dir, path) = temp_path();
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

    #[test]
    fn double_destroy_and_stale_are_errors_not_ub() {
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            let tr = montanha_fdb_transaction_create(db);
            montanha_fdb_database_destroy(db);
            montanha_fdb_database_destroy(db);
            assert_eq!(
                montanha_fdb_transaction_set(tr, b"k".as_ptr(), 1, b"v".as_ptr(), 1),
                MONTAHA_FDB_ERROR
            );
            montanha_fdb_transaction_destroy(tr);
            montanha_fdb_transaction_destroy(tr);
            assert!(montanha_fdb_transaction_create(db).is_null());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_consumes_tr_destroy_is_noop() {
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            let tr = montanha_fdb_transaction_create(db);
            montanha_fdb_transaction_set(tr, b"k".as_ptr(), 1, b"v".as_ptr(), 1);
            assert_eq!(montanha_fdb_transaction_commit(db, tr), MONTAHA_FDB_OK);
            montanha_fdb_transaction_destroy(tr);
            assert_eq!(montanha_fdb_transaction_commit(db, tr), MONTAHA_FDB_ERROR);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mismatched_db_tr_is_error() {
        let (dir, path) = temp_path();
        let dir2 = std::env::temp_dir().join(format!(
            "montanha-fdb-c-b-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir2).unwrap();
        let path2 = CString::new(dir2.to_str().unwrap()).unwrap();
        unsafe {
            let db1 = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            let db2 = montanha_fdb_database_create(path2.as_ptr(), 3, 1);
            let tr = montanha_fdb_transaction_create(db1);
            let mut out = ptr::null_mut();
            let mut len = 0usize;
            assert_eq!(
                montanha_fdb_transaction_get(db2, tr, b"k".as_ptr(), 1, &mut out, &mut len),
                MONTAHA_FDB_ERROR
            );
            assert!(out.is_null());
            assert_eq!(len, 0);
            montanha_fdb_database_destroy(db1);
            montanha_fdb_database_destroy(db2);
        }
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn free_unknown_and_twice_is_noop() {
        unsafe {
            montanha_fdb_free(ptr::null_mut(), 0);
            montanha_fdb_free(0x1 as *mut u8, 4);
            montanha_fdb_free(0x1 as *mut u8, 4);
        }
    }

    #[test]
    fn handle_from_other_thread_is_error_not_ub() {
        let (dir, path) = temp_path();
        let db_bits = unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null());
            db as usize
        };
        let child = std::thread::spawn(move || {
            let db = db_bits as *mut MontanhaFdbDatabase;
            unsafe {
                let tr = montanha_fdb_transaction_create(db);
                assert!(
                    tr.is_null(),
                    "thread-local table: foreign handle must miss, not race"
                );
                montanha_fdb_database_destroy(db);
            }
        });
        child.join().unwrap();
        unsafe {
            let db = db_bits as *mut MontanhaFdbDatabase;
            let tr = montanha_fdb_transaction_create(db);
            assert!(!tr.is_null());
            montanha_fdb_transaction_destroy(tr);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_empty_value_and_null_key_are_defined() {
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            let tr = montanha_fdb_transaction_create(db);
            assert_eq!(
                montanha_fdb_transaction_set(tr, ptr::null(), 0, b"v".as_ptr(), 1),
                MONTAHA_FDB_ERROR
            );
            assert_eq!(
                montanha_fdb_transaction_set(tr, b"k".as_ptr(), 1, ptr::null(), 0),
                MONTAHA_FDB_OK
            );
            assert_eq!(montanha_fdb_transaction_commit(db, tr), MONTAHA_FDB_OK);
            let tr2 = montanha_fdb_transaction_create(db);
            let mut out = ptr::null_mut();
            let mut len = 1usize;
            assert_eq!(
                montanha_fdb_transaction_get(db, tr2, b"k".as_ptr(), 1, &mut out, &mut len),
                MONTAHA_FDB_OK
            );
            assert!(out.is_null());
            assert_eq!(len, 0);
            montanha_fdb_transaction_destroy(tr2);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
