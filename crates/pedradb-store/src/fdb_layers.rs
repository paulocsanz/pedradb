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
            Err(e @ (StoreError::NotCommitted { .. }
            | StoreError::NotLeader { .. }
            | StoreError::Conflict
            | StoreError::TransactionTooOld { .. })) => {
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

    /// Allocate the next name (blind retry).
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn allocate(cluster: &mut StoreCluster) -> Result<Vec<u8>> {
        retry_loop(|| {
            let mut tr = cluster.begin();
            let raw = tr.get(cluster, Self::NEXT)?.unwrap_or_else(|| b"0".to_vec());
            let n: u64 = std::str::from_utf8(&raw)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
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

    /// Allocate or return the name already bound to `token`.
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn allocate(cluster: &mut StoreCluster, token: &[u8]) -> Result<Vec<u8>> {
        let tok = token.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let tkey = {
                let mut k = Self::TOKEN_PREFIX.to_vec();
                k.extend_from_slice(&tok);
                k
            };
            if let Some(existing) = tr.get(cluster, &tkey)? {
                let _ = tr.commit(cluster);
                return Ok(existing);
            }
            let raw = tr.get(cluster, Self::NEXT)?.unwrap_or_else(|| b"0".to_vec());
            let n: u64 = std::str::from_utf8(&raw)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
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
            let mut items = decode_list(&raw);
            items.push(item.clone());
            tr.set(Self::KEY, encode_list(&items))?;
            tr.commit(cluster)?;
            Ok(())
        })
    }

    /// Current items.
    ///
    /// # Errors
    /// Store get errors.
    pub fn items(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        Ok(decode_list(
            tr.get(cluster, Self::KEY)?.unwrap_or_default().as_slice(),
        ))
    }
}

/// Token-idempotent list append.
pub struct SafeList;

impl SafeList {
    /// List blob key.
    pub const KEY: &'static [u8] = b"\x10list";
    /// Seen-token prefix.
    pub const SEEN: &'static [u8] = b"\x11seen/";

    /// Append `item` unless `token` was already applied.
    ///
    /// # Errors
    /// Store errors that are not retryable.
    pub fn append(cluster: &mut StoreCluster, item: &[u8], token: &[u8]) -> Result<()> {
        let item = item.to_vec();
        let token = token.to_vec();
        retry_loop(|| {
            let mut tr = cluster.begin();
            let seen = {
                let mut k = Self::SEEN.to_vec();
                k.extend_from_slice(&token);
                k
            };
            if tr.get(cluster, &seen)?.is_some() {
                let _ = tr.commit(cluster);
                return Ok(());
            }
            let raw = tr.get(cluster, Self::KEY)?.unwrap_or_default();
            let mut items = decode_list(&raw);
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
    /// Store get errors.
    pub fn items(cluster: &StoreCluster) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        Ok(decode_list(
            tr.get(cluster, Self::KEY)?.unwrap_or_default().as_slice(),
        ))
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
            let dkey = {
                let mut k = Self::DATA.to_vec();
                k.extend_from_slice(&key);
                k
            };
            if let Some(old) = tr.get(cluster, &dkey)? {
                if old != value {
                    let mut old_idx = Self::IDX.to_vec();
                    old_idx.extend_from_slice(&old);
                    old_idx.push(b'/');
                    old_idx.extend_from_slice(&key);
                    tr.clear(&old_idx)?;
                }
            }
            tr.set(&dkey, &value)?;
            let mut idx = Self::IDX.to_vec();
            idx.extend_from_slice(&value);
            idx.push(b'/');
            idx.extend_from_slice(&key);
            // Non-empty: Montanha `clear` is an empty-value tombstone (lab).
            tr.set(&idx, b"1")?;
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
        let mut dkey = Self::DATA.to_vec();
        dkey.extend_from_slice(key);
        tr.get(cluster, dkey)
    }

    /// Keys indexed under `value`.
    ///
    /// # Errors
    /// Store range errors.
    pub fn keys_for(cluster: &StoreCluster, value: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut tr = Transaction::at_version(cluster.read_version());
        let mut prefix = Self::IDX.to_vec();
        prefix.extend_from_slice(value);
        prefix.push(b'/');
        let end = crate::prefix_exclusive_end(&prefix).unwrap_or_default();
        let pairs = tr.get_range(cluster, &prefix, &end)?;
        let n = prefix.len();
        Ok(pairs
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(k, _)| k.get(n..).map(Vec::from))
            .collect())
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

fn decode_list(raw: &[u8]) -> Vec<Vec<u8>> {
    let mut items = Vec::new();
    let mut i = 0;
    while i + 8 <= raw.len() {
        let Ok(nhex) = std::str::from_utf8(&raw[i..i + 8]) else {
            break;
        };
        let Ok(n) = usize::from_str_radix(nhex, 16) else {
            break;
        };
        i += 8;
        if i + n > raw.len() {
            break;
        }
        items.push(raw[i..i + n].to_vec());
        i += n;
    }
    items
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
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            let n1 = SafeAllocator::allocate(&mut c, b"tok-a").unwrap();
            assert_eq!(SafeAllocator::names(&c).unwrap().len(), 1);
            // Simulate client lost the Ok (crash after durable commit).
            drop(c);
            let _ = n1;
        }
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        let n2 = SafeAllocator::allocate(&mut c, b"tok-a").unwrap();
        let names = SafeAllocator::names(&c).unwrap();
        assert_eq!(names.len(), 1, "safe token must not leak a name: {names:?}");
        assert_eq!(n2, names[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn naive_allocator_leaks_name_on_crash_retry() {
        let dir = temp();
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            let _ = NaiveAllocator::allocate(&mut c).unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
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
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            IdempotentIndex::put(&mut c, b"k1", b"red").unwrap();
            IdempotentIndex::put(&mut c, b"k2", b"blue").unwrap();
            IdempotentIndex::put(&mut c, b"k1", b"blue").unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(40).unwrap();
        assert_eq!(
            IdempotentIndex::get(&c, b"k1").unwrap().as_deref(),
            Some(b"blue".as_ref())
        );
        let red = IdempotentIndex::keys_for(&c, b"red").unwrap();
        let blue = IdempotentIndex::keys_for(&c, b"blue").unwrap();
        assert!(red.is_empty(), "old index entry must be cleared, got {red:?}");
        assert_eq!(blue.len(), 2, "k1+k2 under blue, got {blue:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Index reverse lookup must include user keys whose first byte is 0xff
    /// (`keys_for` used exclusive end `prefix || 0xff` — same class as F57).
    #[test]
    fn idempotent_index_keys_for_includes_ff_user_key() {
        let dir = temp();
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
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

    #[test]
    fn safe_list_no_dup_on_crash_retry() {
        let dir = temp();
        {
            let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
            c.elect_all(80).unwrap();
            SafeList::append(&mut c, b"item", b"tok-1").unwrap();
            drop(c);
        }
        let mut c = StoreCluster::open(&dir, 3, 1).unwrap();
        c.elect_all(80).unwrap();
        SafeList::append(&mut c, b"item", b"tok-1").unwrap();
        let items = SafeList::items(&c).unwrap();
        assert_eq!(items.len(), 1, "safe list must not duplicate, got {items:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
