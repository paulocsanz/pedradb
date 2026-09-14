//! Thin in-process C ABI (RFC-0023 P2.2). Product face for embedders;
//! **not** FoundationDB / fdbcli / `libfdb_c`.
//!
//! Opaque **handles** (slot + generation packed into the pointer) over
//! [`StoreCluster`] + [`Transaction`]. Double-destroy and use-after-destroy
//! return an error / NULL — they are not `Box::from_raw` UB. Handle tables
//! are **thread-local** (`StoreCluster` is `!Send`). A handle used on
//! another thread yields ERROR, not a data race.
//!
//! Header: `include/montanha_fdb.h`. Build: `cargo build -p pedradb-capi`.
//! Gate: `bash scripts/capi-asan.sh` (honest C PASS; malicious slices
//! ASan-red).
//!
//! Remaining `unsafe` is marshalling only. Lengths are capped before any
//! copy / NUL walk (`MAX_PATH_BYTES` / [`MAX_C_KEY_BYTES`] /
//! [`MAX_C_VALUE_BYTES`]) so a huge `*_len` is `LIMIT`, not a terabyte
//! read. A C caller that lies about a *capped* length is still UB — that
//! is the C contract. Invariants: crate `SAFETY.md`.

#![warn(missing_docs)]

#[path = "handles_kernel.rs"]
mod handles;

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::ptr;

use pedradb_store::client::Transaction;
use pedradb_store::fdb_compat::FdbError;
use pedradb_store::{StoreCluster, StoreError, MAX_TX_BYTES, MAX_VALUE_BYTES};

use handles::{
    c_len_admitted, c_path_nul_off_admitted, c_path_walk_bytes, pack, unpack, Handle, Table,
    KIND_DB, KIND_TX,
};

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

/// Bytes walked looking for a path NUL. No NUL in this window → create
/// returns NULL (no unbounded `strlen`). Bound is `c_path_walk_bytes`.
pub const MAX_PATH_BYTES: usize = handles::C_PATH_WALK_BYTES;
/// Key length cap before copying C bytes (store TX budget).
pub const MAX_C_KEY_BYTES: usize = MAX_TX_BYTES;
/// Value length cap before copying C bytes.
pub const MAX_C_VALUE_BYTES: usize = MAX_VALUE_BYTES;

extern "C" {
    /// Bounded NUL search. Darwin ASan intercepts `memchr` (not `strnlen`).
    fn memchr(s: *const u8, c: i32, n: usize) -> *const u8;
}

fn path_from_c(path: *const c_char) -> Option<PathBuf> {
    if path.is_null() {
        return None;
    }
    // RFC-0075 P1.1: walk bound is the named cap, not a raw 4096 next
    // to `unsafe`. AS-IS `c_path_walk_bytes_as_is` is usize::MAX (strlen).
    let window = c_path_walk_bytes();
    // SAFETY: at most `window` bytes are read. `memchr` is
    // ASan-intercepted: a short heap buffer with no NUL is a C-harness
    // FAIL. A full-window no-NUL buffer is rejected without reading past it.
    let nul = unsafe { memchr(path.cast(), 0, window) };
    if nul.is_null() {
        return None;
    }
    let n = (nul as usize).wrapping_sub(path as usize);
    if !c_path_nul_off_admitted(n) {
        return None;
    }
    let bytes = match copy_c_bytes(path.cast(), n, window) {
        Ok(b) => b,
        Err(_) => return None,
    };
    let s = std::str::from_utf8(&bytes).ok()?;
    Some(PathBuf::from(s))
}

/// Copy `len` bytes from C. `len == 0` is an empty vec (null `p` allowed).
/// `len > max` is [`MONTAHA_FDB_LIMIT`] and does **not** read.
fn copy_c_bytes(p: *const u8, len: usize, max: usize) -> Result<Vec<u8>, c_int> {
    if !c_len_admitted(len, max) {
        return Err(MONTAHA_FDB_LIMIT);
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    if p.is_null() {
        return Err(MONTAHA_FDB_ERROR);
    }
    let mut buf = vec![0u8; len];
    // SAFETY: caller contract — `p` is readable for `len` (≤ `max`).
    // Oversize was rejected above so `SIZE_MAX` never becomes a terabyte
    // claim. `copy_nonoverlapping` is memcpy: ASan intercepts a short
    // buffer in the C harness.
    unsafe {
        ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), len);
    }
    Ok(buf)
}

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
/// `path` is null, or the first [`MAX_PATH_BYTES`] bytes are readable and
/// contain a NUL (C contract). A missing NUL in that window returns NULL.
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_database_create(
    path: *const c_char,
    n_nodes: u64,
    n_ranges: u64,
) -> *mut MontanhaFdbDatabase {
    if path.is_null() || n_nodes == 0 || n_ranges == 0 {
        return ptr::null_mut();
    }
    let Some(p) = path_from_c(path) else {
        return ptr::null_mut();
    };
    match StoreCluster::open(&p, n_nodes, n_ranges) {
        Ok(mut cluster) => {
            // RFC-0067 P2.2: production open is Queued; in-process C face
            // has no Net, so it opts into the unpinned Direct pump.
            cluster.enable_lab_direct_rpc();
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
/// lengths **when those lengths are ≤ the caps**. Oversize is `LIMIT`
/// without a read. `tr` is a handle from create.
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
    if !c_len_admitted(key_len, MAX_C_KEY_BYTES) || !c_len_admitted(value_len, MAX_C_VALUE_BYTES) {
        return MONTAHA_FDB_LIMIT;
    }
    let Some(h) = tx_handle(tr) else {
        return MONTAHA_FDB_ERROR;
    };
    let k = match copy_c_bytes(key, key_len, MAX_C_KEY_BYTES) {
        Ok(k) => k,
        Err(c) => return c,
    };
    let v = match copy_c_bytes(value, value_len, MAX_C_VALUE_BYTES) {
        Ok(v) => v,
        Err(c) => return c,
    };
    with_state(|st| {
        let Some(t) = st.txs.get_mut(h) else {
            return MONTAHA_FDB_ERROR;
        };
        match t.inner.set(&k, &v) {
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
/// `key` readable for `key_len` when `key_len ≤ MAX_C_KEY_BYTES`. Oversize
/// is `LIMIT` (and writes null/0). `out_ptr` / `out_len` live. Handles live
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
    if !c_len_admitted(key_len, MAX_C_KEY_BYTES) {
        // SAFETY: `out_*` checked non-null above.
        unsafe {
            *out_ptr = ptr::null_mut();
            *out_len = 0;
        }
        return MONTAHA_FDB_LIMIT;
    }
    let Some(dh) = db_handle(db) else {
        return MONTAHA_FDB_ERROR;
    };
    let Some(th) = tx_handle(tr) else {
        return MONTAHA_FDB_ERROR;
    };
    let k = match copy_c_bytes(key, key_len, MAX_C_KEY_BYTES) {
        Ok(k) => k,
        Err(c) => {
            unsafe {
                *out_ptr = ptr::null_mut();
                *out_len = 0;
            }
            return c;
        }
    };
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
            t.inner.get(unsafe { &*cluster }, &k)
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
/// RFC-0075 P2.2: this table + `from_raw` is TCB (`c_free_table_admitted`
/// is always false). A twin of `c_len_admitted` is not a free proof.
///
/// # Safety
/// `p` is null or a pointer previously written to `out_ptr` (or garbage,
/// which is ignored).
#[no_mangle]
pub unsafe extern "C" fn montanha_fdb_free(p: *mut u8, len: usize) {
    let _ = len;
    debug_assert!(
        !handles::c_free_table_admitted(),
        "RFC-0075 P2.2: free table is not a proven kernel"
    );
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

    /// RFC-0075 P2.2: get-buffer `free` table stays TCB.
    #[test]
    fn c_free_table_is_tcb() {
        assert!(!handles::c_free_table_admitted());
        assert!(
            handles::c_free_table_admitted_as_is(),
            "AS-IS dente: len-cap twin looks like a free-table proof"
        );
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(
            crate_dir.join("verus/c_len.rs").is_file(),
            "RFC-0075 P2.1: c_len_admitted twin must exist"
        );
        assert!(
            !crate_dir.join("verus/c_free.rs").exists(),
            "RFC-0075 P2.2: free-table Verus twin must stay absent"
        );
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null());
            let tr = montanha_fdb_transaction_create(db);
            assert_eq!(
                montanha_fdb_transaction_set(tr, b"k".as_ptr(), 1, b"v".as_ptr(), 1),
                MONTAHA_FDB_OK
            );
            assert_eq!(montanha_fdb_transaction_commit(db, tr), MONTAHA_FDB_OK);
            let tr2 = montanha_fdb_transaction_create(db);
            let mut out = ptr::null_mut();
            let mut len = 0usize;
            assert_eq!(
                montanha_fdb_transaction_get(db, tr2, b"k".as_ptr(), 1, &mut out, &mut len),
                MONTAHA_FDB_OK
            );
            assert_eq!(len, 1);
            montanha_fdb_free(out, len);
            montanha_fdb_free(out, len);
            montanha_fdb_free(ptr::null_mut(), 0);
            montanha_fdb_free(0x1 as *mut u8, 4);
            montanha_fdb_transaction_destroy(tr2);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
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

    /// RFC-0152 P2.2.38: live C ABI set/get gate `*_len` through
    /// `c_len_admitted`. Oversize value/key is LIMIT; AS-IS would copy any
    /// len. Direct `c_len_oversize_on_live_tx_is_limit` /
    /// `slice_cap_oversize_len_is_limit_without_reading` are not this tooth.
    #[test]
    fn c_len_admitted_on_live_capi_is_not_ok() {
        assert!(!handles::c_len_admitted(
            MAX_C_VALUE_BYTES + 1,
            MAX_C_VALUE_BYTES
        ));
        assert!(
            handles::c_len_admitted_as_is(MAX_C_VALUE_BYTES + 1, MAX_C_VALUE_BYTES),
            "AS-IS dente: copy any len"
        );
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null(), "live database_create");
            let tr = montanha_fdb_transaction_create(db);
            assert!(!tr.is_null(), "live transaction_create");
            let tiny = 1u8;
            assert_eq!(
                montanha_fdb_transaction_set(tr, b"k".as_ptr(), 1, &tiny, MAX_C_VALUE_BYTES + 1),
                MONTAHA_FDB_LIMIT
            );
            let mut out = ptr::null_mut();
            let mut len = 1usize;
            assert_eq!(
                montanha_fdb_transaction_get(
                    db,
                    tr,
                    &tiny,
                    MAX_C_KEY_BYTES + 1,
                    &mut out,
                    &mut len
                ),
                MONTAHA_FDB_LIMIT
            );
            assert!(out.is_null());
            assert_eq!(len, 0);
            montanha_fdb_transaction_destroy(tr);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0075 P0: live C ABI create+tx; oversize key_len is LIMIT and
    /// does not copy. AS-IS would admit the length.
    #[test]
    fn c_len_oversize_on_live_tx_is_limit() {
        assert!(!handles::c_len_admitted(
            MAX_C_KEY_BYTES + 1,
            MAX_C_KEY_BYTES
        ));
        assert!(handles::c_len_admitted_as_is(
            MAX_C_KEY_BYTES + 1,
            MAX_C_KEY_BYTES
        ));
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null(), "live database_create");
            let tr = montanha_fdb_transaction_create(db);
            assert!(!tr.is_null(), "live transaction_create");
            let tiny = 1u8;
            assert_eq!(
                montanha_fdb_transaction_set(tr, &tiny, MAX_C_KEY_BYTES + 1, &tiny, 1),
                MONTAHA_FDB_LIMIT
            );
            montanha_fdb_transaction_destroy(tr);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slice_cap_oversize_len_is_limit_without_reading() {
        // One-byte buffer + huge len must not read (F215). Null handle is
        // fine: the cap fires before table lookup / copy.
        let tiny = 1u8;
        unsafe {
            assert_eq!(
                montanha_fdb_transaction_set(ptr::null_mut(), &tiny, MAX_C_KEY_BYTES + 1, &tiny, 1,),
                MONTAHA_FDB_LIMIT
            );
            assert_eq!(
                montanha_fdb_transaction_set(
                    ptr::null_mut(),
                    &tiny,
                    1,
                    &tiny,
                    MAX_C_VALUE_BYTES + 1,
                ),
                MONTAHA_FDB_LIMIT
            );
            let mut out = ptr::null_mut();
            let mut len = 7usize;
            assert_eq!(
                montanha_fdb_transaction_get(
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &tiny,
                    MAX_C_KEY_BYTES + 1,
                    &mut out,
                    &mut len,
                ),
                MONTAHA_FDB_LIMIT
            );
            assert!(out.is_null());
            assert_eq!(len, 0);
        }
    }

    /// RFC-0156 P0.2 (R-unsafe-capi): boundary sweep over every `*_len`
    /// parameter of the two exports that copy C bytes, on a live
    /// database + tx. Admitted lens (exactly the caps, and 0 with a
    /// null value pointer) are `OK`; oversize (`cap+1`, `usize::MAX`)
    /// is `LIMIT` and must not read (1-byte buffers); null pointers
    /// with len > 0 are `ERROR` before any copy; `get` zeroes the outs
    /// on `LIMIT`. No combination may panic. The AS-IS dente admits
    /// every length.
    #[test]
    fn capi_len_boundary_sweep_on_live_tx() {
        let (dir, path) = temp_path();
        unsafe {
            let db = montanha_fdb_database_create(path.as_ptr(), 3, 1);
            assert!(!db.is_null(), "live database_create");
            let tr = montanha_fdb_transaction_create(db);
            assert!(!tr.is_null(), "live transaction_create");

            let key_cap = vec![1u8; MAX_C_KEY_BYTES];
            let val_cap = vec![2u8; MAX_C_VALUE_BYTES];
            let tiny = 1u8;

            // Oversize key lens: LIMIT, does not read (1-byte buffer) and
            // does not consume the tx budget.
            for bad in [MAX_C_KEY_BYTES + 1, usize::MAX] {
                assert_eq!(
                    montanha_fdb_transaction_set(tr, &tiny, bad, &tiny, 1),
                    MONTAHA_FDB_LIMIT,
                    "oversize key_len {bad:?}"
                );
            }
            // Oversize value lens: LIMIT.
            for bad in [MAX_C_VALUE_BYTES + 1, usize::MAX] {
                assert_eq!(
                    montanha_fdb_transaction_set(tr, &tiny, 1, &tiny, bad),
                    MONTAHA_FDB_LIMIT,
                    "oversize value_len {bad:?}"
                );
            }
            // Null pointers with len > 0: ERROR before any copy.
            assert_eq!(
                montanha_fdb_transaction_set(ptr::null_mut(), ptr::null(), 1, &tiny, 1),
                MONTAHA_FDB_ERROR
            );
            assert_eq!(
                montanha_fdb_transaction_set(tr, ptr::null(), 1, &tiny, 1),
                MONTAHA_FDB_ERROR
            );
            assert_eq!(
                montanha_fdb_transaction_set(tr, &tiny, 1, ptr::null(), 1),
                MONTAHA_FDB_ERROR
            );

            let mut out_ptr: *mut u8 = ptr::null_mut();
            let mut out_len: usize = 7;
            // get: oversize key len is LIMIT and zeroes the outs.
            for bad in [MAX_C_KEY_BYTES + 1, usize::MAX] {
                out_len = 7;
                assert_eq!(
                    montanha_fdb_transaction_get(db, tr, &tiny, bad, &mut out_ptr, &mut out_len),
                    MONTAHA_FDB_LIMIT,
                    "oversize get key_len {bad:?}"
                );
                assert!(out_ptr.is_null() && out_len == 0);
            }
            // get: null key / null outs are ERROR.
            assert_eq!(
                montanha_fdb_transaction_get(db, tr, ptr::null(), 1, &mut out_ptr, &mut out_len),
                MONTAHA_FDB_ERROR
            );
            assert_eq!(
                montanha_fdb_transaction_get(db, tr, &tiny, 1, ptr::null_mut(), &mut out_len),
                MONTAHA_FDB_ERROR
            );
            assert_eq!(
                montanha_fdb_transaction_get(db, tr, &tiny, 1, &mut out_ptr, ptr::null_mut()),
                MONTAHA_FDB_ERROR
            );

            // Admitted boundaries. The tx budget is cumulative (key+value
            // per tx), so each at-cap case gets its own fresh tx.
            let tr_k = montanha_fdb_transaction_create(db);
            assert!(!tr_k.is_null());
            assert_eq!(
                montanha_fdb_transaction_set(
                    tr_k,
                    key_cap.as_ptr(),
                    MAX_C_KEY_BYTES,
                    ptr::null(),
                    0,
                ),
                MONTAHA_FDB_OK,
                "key exactly at the cap is admitted"
            );
            montanha_fdb_transaction_destroy(tr_k);

            let tr_v = montanha_fdb_transaction_create(db);
            assert!(!tr_v.is_null());
            assert_eq!(
                montanha_fdb_transaction_set(tr_v, &tiny, 1, val_cap.as_ptr(), MAX_C_VALUE_BYTES),
                MONTAHA_FDB_OK,
                "value exactly at the cap is admitted"
            );
            // Admitted: len 0 with a null value pointer is the empty value.
            assert_eq!(
                montanha_fdb_transaction_set(tr_v, &tiny, 1, ptr::null(), 0),
                MONTAHA_FDB_OK
            );
            montanha_fdb_transaction_destroy(tr_v);

            // Dente: AS-IS admits every length.
            assert!(handles::c_len_admitted_as_is(usize::MAX, MAX_C_KEY_BYTES));

            montanha_fdb_transaction_destroy(tr);
            montanha_fdb_database_destroy(db);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slice_cap_path_without_nul_in_max_is_null() {
        let buf = vec![b'a'; MAX_PATH_BYTES];
        unsafe {
            assert!(montanha_fdb_database_create(buf.as_ptr().cast(), 3, 1).is_null());
        }
    }

    /// RFC-0075 P1.1: path NUL walk uses the named `MAX_PATH_BYTES` cap.
    /// AS-IS would walk `usize::MAX`.
    #[test]
    fn c_path_walk_uses_named_cap() {
        assert_eq!(handles::c_path_walk_bytes(), MAX_PATH_BYTES);
        assert_eq!(MAX_PATH_BYTES, 4096, "header MONTAHA_FDB_MAX_PATH_BYTES");
        assert_eq!(handles::c_path_walk_bytes_as_is(), usize::MAX);
        assert!(handles::c_path_nul_off_admitted(0));
        assert!(handles::c_path_nul_off_admitted(MAX_PATH_BYTES - 1));
        assert!(!handles::c_path_nul_off_admitted(MAX_PATH_BYTES));
        assert!(
            handles::c_path_nul_off_admitted_as_is(MAX_PATH_BYTES),
            "AS-IS dente: offset past the window"
        );
        let buf = vec![b'a'; MAX_PATH_BYTES];
        unsafe {
            assert!(
                montanha_fdb_database_create(buf.as_ptr().cast(), 3, 1).is_null(),
                "no NUL in the named window is NULL"
            );
        }
    }
}
