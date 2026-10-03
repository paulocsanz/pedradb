//! FDB-shaped application layers on Montanha (directory / list / index).
//!
//! These are the same layers `fdb-dst` hunts against FoundationDB. The store
//! is Montanha — not a mock and not `libfdb_c`. Naive* = blind retry after
//! `NotCommitted` / reopen (the 1021 leak class). Safe* = client token.

use crate::client::Transaction;
use crate::{Result, StoreCluster, StoreError};

/// Blind retry budget (FDB 1021-shaped). Unbounded Conflict retry hangs soaks.
const LAYER_RETRY_LIMIT: u32 = 64;

/// Retry on retryable store errors (the careless layer).
fn retry_loop<T>(mut once: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last: Option<StoreError> = None;
    for _ in 0..LAYER_RETRY_LIMIT {
        match once() {
            Ok(v) => return Ok(v),
            Err(
                e @ (StoreError::NotCommitted { .. }
                | StoreError::NotLeader { .. }
                | StoreError::Conflict
                | StoreError::TransactionTooOld { .. }),
            ) => {
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| StoreError::Msg("layer retry budget exceeded".into())))
}

/// Directory-shaped: increment a counter, write `name/N`. Blind retry leaks a name.
pub struct NaiveAllocator;

impl NaiveAllocator {
    /// Counter key.
    pub const NEXT: &'static [u8] = b"\x01next";
    /// Allocated-name prefix.
    pub const NAME_PREFIX: &'static [u8] = b"\x02";

    /// Parse `NEXT` counter. Missing key → 0; present but corrupt → hard error (F109).
    fn parse_next(raw: Option<Vec<u8>>) -> Result<u64> {
        match raw {
            None => Ok(0),
            Some(raw) => {
                let s = std::str::from_utf8(&raw)
                    .map_err(|_| StoreError::Msg("allocator NEXT: bad utf8".into()))?;
                s.parse::<u64>()
                    .map_err(|_| StoreError::Msg("allocator NEXT: bad integer".into()))
            }
        }
    }

    /// Allocate the next name (blind retry).
    ///
    /// # Errors
    /// Store errors that are not retryable; corrupt `NEXT` (F109).
    pub fn allocate(cluster: &mut StoreCluster) -> Result<Vec<u8>> {
        retry_loop(|| {
            let mut tr = cluster.begin();
            let n = Self::parse_next(tr.get(cluster, Self::NEXT)?)?;
            let name = {
                let mut k = Self::NAME_PREFIX.to_vec();
                k.extend_from_slice(format!("{n:016}").as_bytes());
                k
            };
            tr.set(Self::NEXT, (n + 1).to_string().into_bytes())?;
            tr.set(&name, b"allocated")?;
            tr.commit(cluster)?;
            Ok(name)
        })
    }

    /// Names currently stored.
    ///
    /// # Errors
    /// Store get_range errors.
    pub fn names(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let pairs = tr.get_range(cluster, Self::NAME_PREFIX, b"\x03")?;
        Ok(pairs.into_iter().map(|(k, _)| k).collect())
    }
}

/// Same shape, but the client supplies a token and re-reads it (no leak).
pub struct SafeAllocator;

impl SafeAllocator {
    /// Counter key.
    pub const NEXT: &'static [u8] = b"\x01next";
    /// Allocated-name prefix.
    pub const NAME_PREFIX: &'static [u8] = b"\x02";
    /// Token → name.
    pub const TOKEN_PREFIX: &'static [u8] = b"\x03";

    /// F95: token map key `TOKEN || u32be(len) || token` (raw concat made
    /// `TOKEN||a` a prefix of `TOKEN||ab`).
    fn token_key(token: &[u8]) -> Vec<u8> {
        let mut k = Self::TOKEN_PREFIX.to_vec();
        let n = u32::try_from(token.len()).expect("token len fits u32");
        k.extend_from_slice(&n.to_be_bytes());
        k.extend_from_slice(token);
        k
    }

    /// Allocate or return the name already bound to `token`.
    ///
    /// # Errors
    /// Store errors that are not retryable; corrupt `NEXT` (F109).
    pub fn allocate(cluster: &mut StoreCluster, token: &[u8]) -> Result<Vec<u8>> {
        let tok = token.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let tkey = Self::token_key(&tok);
            if let Some(existing) = tr.get(cluster, &tkey)? {
                let _ = tr.commit(cluster);
                return Ok(existing);
            }
            // F109: same fail-closed NEXT parse as NaiveAllocator.
            let n = NaiveAllocator::parse_next(tr.get(cluster, Self::NEXT)?)?;
            let name = {
                let mut k = Self::NAME_PREFIX.to_vec();
                k.extend_from_slice(format!("{n:016}").as_bytes());
                k
            };
            tr.set(Self::NEXT, (n + 1).to_string().into_bytes())?;
            tr.set(&name, &tok)?;
            tr.set(&tkey, &name)?;
            tr.commit(cluster)?;
            Ok(name)
        })
    }

    /// Names currently stored.
    ///
    /// # Errors
    /// Store get_range errors.
    pub fn names(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let pairs = tr.get_range(cluster, Self::NAME_PREFIX, b"\x03")?;
        Ok(pairs.into_iter().map(|(k, _)| k).collect())
    }
}

/// Read-modify-write list. Blind retry duplicates the item.
pub struct NaiveList;

impl NaiveList {
    /// List blob key.
    pub const KEY: &'static [u8] = b"\x10list";

    /// Append `item` (blind retry).
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn append(cluster: &mut StoreCluster, item: &[u8]) -> Result<()> {
        let item = item.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let raw = tr.get(cluster, Self::KEY)?.unwrap_or_default();
            let mut items = decode_list(&raw)?;
            items.push(item.clone());
            tr.set(Self::KEY, encode_list(&items))?;
            tr.commit(cluster)?;
            Ok(())
        })
    }

    /// Current items.
    ///
    /// # Errors
    /// Store get errors or corrupt list encoding (F107).
    pub fn items(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        decode_list(tr.get(cluster, Self::KEY)?.unwrap_or_default().as_slice())
    }
}

/// Token-idempotent list append.
pub struct SafeList;

impl SafeList {
    /// List blob key.
    pub const KEY: &'static [u8] = b"\x10list";
    /// Seen-token prefix.
    pub const SEEN: &'static [u8] = b"\x11seen/";

    /// F94: seen keys are `SEEN || u32be(len) || token` so `SEEN||a` is not a
    /// byte-prefix of `SEEN||ab` under half-open scans.
    fn seen_key(token: &[u8]) -> Vec<u8> {
        let mut k = Self::SEEN.to_vec();
        let n = u32::try_from(token.len()).expect("token len fits u32");
        k.extend_from_slice(&n.to_be_bytes());
        k.extend_from_slice(token);
        k
    }

    /// Append `item` unless `token` was already applied.
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn append(cluster: &mut StoreCluster, item: &[u8], token: &[u8]) -> Result<()> {
        let item = item.to_vec();
        let token = token.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let seen = Self::seen_key(&token);
            if tr.get(cluster, &seen)?.is_some() {
                let _ = tr.commit(cluster);
                return Ok(());
            }
            let raw = tr.get(cluster, Self::KEY)?.unwrap_or_default();
            let mut items = decode_list(&raw)?;
            items.push(item.clone());
            tr.set(Self::KEY, encode_list(&items))?;
            tr.set(&seen, b"1")?;
            tr.commit(cluster)?;
            Ok(())
        })
    }

    /// Current items.
    ///
    /// # Errors
    /// Store get errors or corrupt list encoding (F107).
    pub fn items(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        decode_list(tr.get(cluster, Self::KEY)?.unwrap_or_default().as_slice())
    }
}

/// Deterministic secondary index (survives maybe-committed retry).
pub struct IdempotentIndex;

impl IdempotentIndex {
    /// Data prefix.
    pub const DATA: &'static [u8] = b"\x20d/";
    /// Index prefix.
    pub const IDX: &'static [u8] = b"\x21i/";

    /// Put `key → value` and maintain the reverse index.
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn put(cluster: &mut StoreCluster, key: &[u8], value: &[u8]) -> Result<()> {
        let key = key.to_vec();
        let value = value.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let dkey = Self::data_key(&key);
            if let Some(old) = tr.get(cluster, &dkey)? {
                if old != value {
                    tr.clear(Self::idx_key(&old, &key))?;
                }
            }
            tr.set(&dkey, &value)?;
            // Non-empty: Montanha `clear` is an empty-value tombstone (lab).
            tr.set(Self::idx_key(&value, &key), b"1")?;
            tr.commit(cluster)?;
            Ok(())
        })
    }

    /// Get data key.
    ///
    /// # Errors
    /// Store get errors.
    pub fn get(cluster: &StoreCluster, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        tr.get(cluster, Self::data_key(key))
    }

    /// Keys indexed under `value`.
    ///
    /// Reverse keys are
    /// `IDX || u32be(len(value)) || value || 0x00 || u32be(len(key)) || key`
    /// (F78 value; F93 user key length-prefix).
    ///
    /// # Errors
    /// Store range errors.
    pub fn keys_for(cluster: &StoreCluster, value: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let (start, end) = Self::idx_children(value);
        let pairs = tr.get_range(cluster, &start, &end)?;
        let n = start.len();
        Ok(pairs
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(k, _)| {
                let rest = k.get(n..)?;
                // F93: length-prefixed user key after the value child sep.
                if rest.len() < 4 {
                    return None;
                }
                let ulen = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
                if rest.len() != 4 + ulen {
                    // Legacy raw user key (pre-F93): accept remainder as key.
                    return Some(rest.to_vec());
                }
                Some(rest[4..].to_vec())
            })
            .collect())
    }

    fn push_len(buf: &mut Vec<u8>, part: &[u8]) {
        let n = u32::try_from(part.len()).expect("index component len fits u32");
        buf.extend_from_slice(&n.to_be_bytes());
        buf.extend_from_slice(part);
    }

    /// F93: data keys are `DATA || u32be(len) || key` so `DATA||a` is not a
    /// byte-prefix of `DATA||ab` under half-open scans.
    fn data_key(key: &[u8]) -> Vec<u8> {
        let mut k = Self::DATA.to_vec();
        Self::push_len(&mut k, key);
        k
    }

    fn idx_prefix(value: &[u8]) -> Vec<u8> {
        let mut idx = Self::IDX.to_vec();
        idx.extend_from_slice(&crate::len_pref_value(value));
        idx
    }

    fn idx_key(value: &[u8], key: &[u8]) -> Vec<u8> {
        let mut idx = Self::idx_prefix(value);
        idx.push(0x00);
        // F93: length-prefix user key (was raw concat).
        Self::push_len(&mut idx, key);
        idx
    }

    /// `[IDX||len||value||0x00, IDX||len||value||0x01)` — exact value, any user key.
    fn idx_children(value: &[u8]) -> (Vec<u8>, Vec<u8>) {
        crate::exact_value_children(&Self::idx_prefix(value))
    }
}

fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for it in items {
        out.extend_from_slice(format!("{:08x}", it.len()).as_bytes());
        out.extend_from_slice(it);
    }
    out
}

/// F107: truncated / corrupt list blobs used to return a *prefix* of items
/// (`break` on bad hex or short payload). Append then persisted that prefix
/// and dropped the tail — silent loss. Fail closed.
fn decode_list(raw: &[u8]) -> Result<Vec<Vec<u8>>> {
    if pedradb_core::write_admission_kernel::batch_is_empty(raw.len() as u64) {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if i + 8 > raw.len() {
            return Err(StoreError::Msg("list decode: truncated length".into()));
        }
        let nhex = std::str::from_utf8(&raw[i..i + 8])
            .map_err(|_| StoreError::Msg("list decode: bad length utf8".into()))?;
        let n = usize::from_str_radix(nhex, 16)
            .map_err(|_| StoreError::Msg("list decode: bad length hex".into()))?;
        i += 8;
        if i + n > raw.len() {
            return Err(StoreError::Msg("list decode: truncated item".into()));
        }
        items.push(raw[i..i + n].to_vec());
        i += n;
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreCluster;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let i = N.fetch_add(1, Ordering::Relaxed);
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("fdb-layers-{n}-{i}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn safe_allocator_exact_after_crash_retry() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            let n1 = SafeAllocator::allocate(&mut c, b"tok-a").unwrap();
            assert_eq!(SafeAllocator::names(&c).unwrap().len(), 1);
            // Simulate client lost the Ok (crash after durable commit).
            drop(c);
            let _ = n1;
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let n2 = SafeAllocator::allocate(&mut c, b"tok-a").unwrap();
        let names = SafeAllocator::names(&c).unwrap();
        assert_eq!(names.len(), 1, "safe token must not leak a name: {names:?}");
        assert_eq!(n2, names[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F109: garbage `NEXT` parsed as 0 and reused the first name slot.
    #[test]
    fn allocator_corrupt_next_does_not_reuse_name_zero() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let n0 = NaiveAllocator::allocate(&mut c).unwrap();
        c.put(NaiveAllocator::NEXT, b"xxx").unwrap();
        let err = NaiveAllocator::allocate(&mut c);
        assert!(
            err.is_err(),
            "corrupt NEXT must fail closed, not reuse slot 0: {err:?}"
        );
        assert_eq!(
            c.get(&n0).unwrap().as_deref(),
            Some(b"allocated".as_ref()),
            "first allocation must survive corrupt NEXT"
        );
        // Heal counter so SafeAllocator can prove the same contract on its path
        // (Naive/Safe share NEXT key bytes in this recipe).
        c.put(NaiveAllocator::NEXT, b"1").unwrap();
        let s0 = SafeAllocator::allocate(&mut c, b"tok-0").unwrap();
        c.put(SafeAllocator::NEXT, b"yyy").unwrap();
        let err = SafeAllocator::allocate(&mut c, b"tok-1");
        assert!(
            err.is_err(),
            "SafeAllocator corrupt NEXT must fail closed: {err:?}"
        );
        assert_eq!(
            c.get(&s0).unwrap().as_deref(),
            Some(b"tok-0".as_ref()),
            "safe first name must survive"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn naive_allocator_leaks_name_on_crash_retry() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            let _ = NaiveAllocator::allocate(&mut c).unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let _ = NaiveAllocator::allocate(&mut c).unwrap();
        let names = NaiveAllocator::names(&c).unwrap();
        // This is the FDB 1021 class: the *layer* leaks, the store is correct.
        assert_eq!(
            names.len(),
            2,
            "naive retry after unseen Ok must leak a name (fonte), got {names:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idempotent_index_consistent_after_reopen() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            IdempotentIndex::put(&mut c, b"k1", b"red").unwrap();
            IdempotentIndex::put(&mut c, b"k2", b"blue").unwrap();
            IdempotentIndex::put(&mut c, b"k1", b"blue").unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, b"k1").unwrap().as_deref(),
            Some(b"blue".as_ref())
        );
        let red = IdempotentIndex::keys_for(&c, b"red").unwrap();
        let blue = IdempotentIndex::keys_for(&c, b"blue").unwrap();
        assert!(
            red.is_empty(),
            "old index entry must be cleared, got {red:?}"
        );
        assert_eq!(blue.len(), 2, "k1+k2 under blue, got {blue:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Index reverse lookup must include user keys whose first byte is 0xff
    /// (`keys_for` used exclusive end `prefix || 0xff` — same class as F57).
    #[test]
    fn idempotent_index_keys_for_includes_ff_user_key() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let ff = [0xff, b'z'];
        IdempotentIndex::put(&mut c, b"plain", b"red").unwrap();
        IdempotentIndex::put(&mut c, &ff, b"red").unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, &ff).unwrap().as_deref(),
            Some(b"red".as_ref()),
            "point get must see 0xff key"
        );
        let red = IdempotentIndex::keys_for(&c, b"red").unwrap();
        assert!(
            red.iter().any(|k| k.as_slice() == b"plain"),
            "plain key missing: {red:?}"
        );
        assert!(
            red.iter().any(|k| k.as_slice() == ff),
            "0xff user key missing from keys_for (prefix||0xff end): {red:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reverse keys were `IDX || value || '/' || user_key`. `keys_for("red")`
    /// used prefix `IDX||red||/` whose exclusive successor still contains
    /// `IDX||red/foo||/…` (`'/' < successor('/')`).
    #[test]
    fn idempotent_index_keys_for_does_not_include_value_prefix_sibling() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        IdempotentIndex::put(&mut c, b"k1", b"red").unwrap();
        IdempotentIndex::put(&mut c, b"k2", b"red/foo").unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, b"k2").unwrap().as_deref(),
            Some(b"red/foo".as_ref())
        );
        let red = IdempotentIndex::keys_for(&c, b"red").unwrap();
        assert!(
            red.iter().any(|k| k.as_slice() == b"k1"),
            "k1 missing under red: {red:?}"
        );
        assert!(
            !red.iter()
                .any(|k| k.as_slice() == b"k2" || k.windows(3).any(|w| w == b"foo")),
            "keys_for(red) included sibling value red/foo: {red:?}"
        );
        let long = IdempotentIndex::keys_for(&c, b"red/foo").unwrap();
        assert_eq!(long, vec![b"k2".to_vec()], "exact value red/foo: {long:?}");
        let nul_val = [b'r', 0x00, b'd'];
        let nul_key = [b'k', 0x00, b'3'];
        IdempotentIndex::put(&mut c, &nul_key, &nul_val).unwrap();
        let got = IdempotentIndex::keys_for(&c, &nul_val).unwrap();
        assert!(
            got.iter().any(|k| k.as_slice() == nul_key),
            "NUL in value/key must round-trip: {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F78: `IDX||value||0x00||key` is not injective on `value`.
    ///
    /// `keys_for("red")` scans `[IDX||red||0x00, IDX||red||0x01)`. Value
    /// `red||0x00||foo` encodes as `IDX||red||0x00||foo||0x00||k2`, which
    /// lives inside that interval — ghost user key `foo||0x00||k2`.
    #[test]
    fn idempotent_index_keys_for_does_not_include_nul_value_prefix_sibling() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        IdempotentIndex::put(&mut c, b"k1", b"red").unwrap();
        let long_val = [b'r', b'e', b'd', 0x00, b'f', b'o', b'o'];
        IdempotentIndex::put(&mut c, b"k2", &long_val).unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, b"k2").unwrap().as_deref(),
            Some(long_val.as_slice())
        );
        let red = IdempotentIndex::keys_for(&c, b"red").unwrap();
        assert!(
            red.iter().any(|k| k.as_slice() == b"k1"),
            "k1 missing under red: {red:?}"
        );
        assert!(
            !red.iter().any(|k| k.as_slice() == b"k2"
                || k.windows(3).any(|w| w == b"foo")
                || k.contains(&0x00)),
            "keys_for(red) included sibling value red\\0foo: {red:?}"
        );
        let long = IdempotentIndex::keys_for(&c, &long_val).unwrap();
        assert_eq!(
            long,
            vec![b"k2".to_vec()],
            "exact value red\\0foo: {long:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F93: data keys were `DATA || key` so `DATA||a` is a byte-prefix of
    /// `DATA||ab` — a half-open prefix scan of one id leaked the sibling.
    #[test]
    fn idempotent_index_data_key_not_prefix_of_sibling_id() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        IdempotentIndex::put(&mut c, b"a", b"va").unwrap();
        IdempotentIndex::put(&mut c, b"ab", b"vab").unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, b"a").unwrap().as_deref(),
            Some(b"va".as_ref())
        );
        assert_eq!(
            IdempotentIndex::get(&c, b"ab").unwrap().as_deref(),
            Some(b"vab".as_ref())
        );
        let dk_a = IdempotentIndex::data_key(b"a");
        let dk_ab = IdempotentIndex::data_key(b"ab");
        assert!(
            !dk_ab.starts_with(&dk_a),
            "data_key(a) must not be a prefix of data_key(ab): {dk_a:?} vs {dk_ab:?}"
        );
        let end = crate::prefix_exclusive_end(&dk_a);
        let hits = c
            .keys_in_range_at(&dk_a, end.as_deref().unwrap_or(&[]), c.read_version())
            .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "prefix scan of data_key(a) leaked siblings: {hits:?}"
        );
        assert_eq!(hits[0].0, dk_a);
        // Reverse index still lists exact user keys.
        assert_eq!(
            IdempotentIndex::keys_for(&c, b"va").unwrap(),
            vec![b"a".to_vec()]
        );
        assert_eq!(
            IdempotentIndex::keys_for(&c, b"vab").unwrap(),
            vec![b"ab".to_vec()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F107: truncated tail used to decode as a silent prefix (lost last item).
    #[test]
    fn decode_list_rejects_truncated_tail() {
        let raw = encode_list(&[b"aa".to_vec(), b"bb".to_vec()]);
        assert_eq!(
            decode_list(&raw).unwrap(),
            vec![b"aa".to_vec(), b"bb".to_vec()]
        );
        assert!(
            decode_list(b"").unwrap().is_empty(),
            "empty blob is empty list"
        );
        let short = &raw[..raw.len() - 1];
        assert!(
            decode_list(short).is_err(),
            "truncated list must fail closed, got {:?}",
            decode_list(short)
        );
        let garbage = [raw.as_slice(), b"xx"].concat();
        assert!(
            decode_list(&garbage).is_err(),
            "trailing garbage must fail closed"
        );
    }

    #[test]
    fn safe_list_no_dup_on_crash_retry() {
        let dir = temp();
        {
            let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            SafeList::append(&mut c, b"item", b"tok-1").unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        SafeList::append(&mut c, b"item", b"tok-1").unwrap();
        let items = SafeList::items(&c).unwrap();
        assert_eq!(
            items.len(),
            1,
            "safe list must not duplicate, got {items:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F94: seen keys were `SEEN || token` so `SEEN||a` is a prefix of `SEEN||ab`.
    #[test]
    fn safe_list_seen_token_not_prefix_of_sibling() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        SafeList::append(&mut c, b"i1", b"a").unwrap();
        SafeList::append(&mut c, b"i2", b"ab").unwrap();
        assert_eq!(SafeList::items(&c).unwrap().len(), 2);
        let sa = SafeList::seen_key(b"a");
        let sab = SafeList::seen_key(b"ab");
        assert!(
            !sab.starts_with(&sa),
            "seen_key(a) must not prefix seen_key(ab): {sa:?} vs {sab:?}"
        );
        // Distinct tokens stay independent: re-append `a` is still idempotent.
        SafeList::append(&mut c, b"i1-dup", b"a").unwrap();
        assert_eq!(
            SafeList::items(&c).unwrap().len(),
            2,
            "token a must not collide with ab"
        );
        let end = crate::prefix_exclusive_end(&sa);
        let hits = c
            .keys_in_range_at(&sa, end.as_deref().unwrap_or(&[]), c.read_version())
            .unwrap();
        assert_eq!(hits.len(), 1, "prefix scan of seen_key(a) leaked: {hits:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F95: token keys were `TOKEN || token` so `TOKEN||a` prefixes `TOKEN||ab`.
    #[test]
    fn safe_allocator_token_not_prefix_of_sibling() {
        let dir = temp();
        let mut c = StoreCluster::open_lab_direct(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let n1 = SafeAllocator::allocate(&mut c, b"a").unwrap();
        let n2 = SafeAllocator::allocate(&mut c, b"ab").unwrap();
        assert_ne!(n1, n2, "distinct tokens must get distinct names");
        let t1 = SafeAllocator::token_key(b"a");
        let t2 = SafeAllocator::token_key(b"ab");
        assert!(
            !t2.starts_with(&t1),
            "token_key(a) must not prefix token_key(ab): {t1:?} vs {t2:?}"
        );
        // Re-allocate with token `a` must return same name (not ab's).
        let n1b = SafeAllocator::allocate(&mut c, b"a").unwrap();
        assert_eq!(n1b, n1);
        let end = crate::prefix_exclusive_end(&t1);
        let hits = c
            .keys_in_range_at(&t1, end.as_deref().unwrap_or(&[]), c.read_version())
            .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "prefix scan of token_key(a) leaked: {hits:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
