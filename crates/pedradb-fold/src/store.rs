//! Pedra-backed [`FoldStore`].

use crate::{FoldCursor, FoldError, FoldMetrics, FoldRole, FoldUpdate, Result};
use bytes::Bytes;
use pedradb_core::{prefix_exclusive_end, BatchOp, Db, Env, OpenOptions, StdEnv};
use std::path::{Path, PathBuf};

/// Meta key for the applied cursor (not a user key).
pub const CURSOR_KEY: &[u8] = b"\0fold/cursor";
/// Prefix for "key present" markers (proxy keep-keys).
pub const KEYSET_PREFIX: &[u8] = b"\0fold/keyset/";

/// Durable fold + cursor + query.
pub trait FoldStore {
    /// Open or create at `path`. Returns the recovered cursor.
    ///
    /// # Errors
    /// Pedra open / I/O.
    fn open(path: &Path) -> Result<(FoldCursor, Self)>
    where
        Self: Sized;

    /// Apply `batch` and advance cursor **atomically** (one Pedra apply_batch).
    ///
    /// # Errors
    /// Pedra apply. On error the cursor must remain at the previous pin.
    fn apply(&mut self, batch: &[FoldUpdate], cursor: FoldCursor) -> Result<()>;

    /// Point get (LocalApplied). Proxy with evicted value returns `None`.
    ///
    /// # Errors
    /// Store I/O (none today on Pedra get).
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Ordered prefix scan of live user keys (not meta).
    ///
    /// # Errors
    /// Store I/O.
    fn range(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;

    /// Current applied cursor.
    fn cursor(&self) -> FoldCursor;
}

/// Pedra directory fold.
pub struct PedraFold<E: Env = StdEnv> {
    db: Db<E>,
    dir: PathBuf,
    cursor: FoldCursor,
    role: FoldRole,
    /// Telemetry.
    pub metrics: FoldMetrics,
}

impl PedraFold<StdEnv> {
    /// Open with an explicit role on [`StdEnv`].
    ///
    /// # Errors
    /// Pedra open.
    pub fn open_role(path: &Path, role: FoldRole) -> Result<(FoldCursor, Self)> {
        Self::open_role_env(path, role, StdEnv)
    }
}

impl<E: Env> PedraFold<E> {
    /// Open with an explicit [`Env`] (FailingEnv soak).
    ///
    /// # Errors
    /// Pedra open.
    pub fn open_role_env(path: &Path, role: FoldRole, env: E) -> Result<(FoldCursor, Self)> {
        let opts = OpenOptions {
            sync: true,
            auto_flush_bytes: None,
            auto_compact_sst_count: None,
            auto_compact_sst_bytes: None,
            exclusive: true,
            large_value_threshold: None,
        };
        let db = Db::open_with_env(path, opts, env)?;
        let cursor = match db.get(CURSOR_KEY) {
            Some(raw) if raw.len() == 8 => {
                let mut b = [0u8; 8];
                b.copy_from_slice(&raw);
                FoldCursor(u64::from_le_bytes(b))
            }
            _ => FoldCursor::none(),
        };
        let fold = Self {
            db,
            dir: path.to_path_buf(),
            cursor,
            role,
            metrics: FoldMetrics {
                fold_cursor: cursor.0,
                ..FoldMetrics::default()
            },
        };
        Ok((cursor, fold))
    }

    /// Directory of this fold.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Role.
    #[must_use]
    pub fn role(&self) -> FoldRole {
        self.role
    }

    /// Applied cursor (inherent; also [`FoldStore::cursor`] on StdEnv).
    #[must_use]
    pub fn applied_cursor(&self) -> FoldCursor {
        self.cursor
    }

    /// Whether `key` is known (proxy: present even if value evicted).
    #[must_use]
    pub fn contains_key(&self, key: &[u8]) -> bool {
        if self.db.get(keyset_key(key).as_slice()).is_some() {
            return true;
        }
        self.db.get(key).is_some()
    }

    /// Evict values for `keys` (proxy). Keys stay.
    ///
    /// # Errors
    /// Apply.
    pub fn evict_values(&mut self, keys: &[&[u8]]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut ops = Vec::new();
        for k in keys {
            ops.push(BatchOp::put(keyset_key(k), Bytes::new()));
            ops.push(BatchOp::put(*k, Bytes::new()));
        }
        self.db.apply_batch(ops)?;
        Ok(())
    }

    /// Mutable Pedra handle (export/checkpoint).
    pub fn db_mut(&mut self) -> &mut Db<E> {
        &mut self.db
    }

    /// Apply batch (same as [`FoldStore::apply`]; used with non-Std env).
    ///
    /// # Errors
    /// Pedra apply.
    pub fn apply_updates(&mut self, batch: &[FoldUpdate], cursor: FoldCursor) -> Result<()> {
        apply_inner(self, batch, cursor)
    }

    /// Close exclusive lock so another open can import/export.
    pub fn close(self) {
        drop(self.db);
    }
}

fn apply_inner<E: Env>(
    fold: &mut PedraFold<E>,
    batch: &[FoldUpdate],
    cursor: FoldCursor,
) -> Result<()> {
    if matches!(fold.role, FoldRole::Relay) {
        fold.db.apply_batch([BatchOp::put(
            CURSOR_KEY,
            Bytes::copy_from_slice(&cursor.0.to_le_bytes()),
        )])?;
        fold.cursor = cursor;
        fold.metrics.fold_cursor = cursor.0;
        fold.metrics.fold_apply_batches = fold.metrics.fold_apply_batches.saturating_add(1);
        return Ok(());
    }
    let evict = matches!(fold.role, FoldRole::Proxy { evict_values: true });
    let mut ops: Vec<BatchOp> = Vec::new();
    for u in batch {
        match u {
            FoldUpdate::Put { key, value, .. } => {
                // F69: never accept user keys under `\0fold/` (would clobber cursor/keyset).
                if crate::follow::is_fold_meta_key(key) {
                    return Err(FoldError::TransientApply(
                        "user key reserved for fold meta (\\0fold/)".into(),
                    ));
                }
                ops.push(BatchOp::put(keyset_key(key), Bytes::new()));
                if evict {
                    ops.push(BatchOp::put(key.as_slice(), Bytes::new()));
                } else {
                    ops.push(BatchOp::put(key.as_slice(), value.as_slice()));
                }
            }
            FoldUpdate::Delete { key, .. } => {
                if crate::follow::is_fold_meta_key(key) {
                    return Err(FoldError::TransientApply(
                        "user key reserved for fold meta (\\0fold/)".into(),
                    ));
                }
                ops.push(BatchOp::delete(key.as_slice()));
                ops.push(BatchOp::delete(keyset_key(key)));
            }
        }
    }
    ops.push(BatchOp::put(
        CURSOR_KEY,
        Bytes::copy_from_slice(&cursor.0.to_le_bytes()),
    ));
    match fold.db.apply_batch(ops) {
        Ok(_) => {
            fold.cursor = cursor;
            fold.metrics.fold_cursor = cursor.0;
            fold.metrics.fold_apply_batches = fold.metrics.fold_apply_batches.saturating_add(1);
            Ok(())
        }
        Err(e) => {
            fold.metrics.fold_apply_err = fold.metrics.fold_apply_err.saturating_add(1);
            Err(FoldError::TransientApply(e.to_string()))
        }
    }
}

impl<E: Env> PedraFold<E> {
    fn get_user(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if matches!(self.role, FoldRole::Relay) {
            return Ok(None);
        }
        if matches!(self.role, FoldRole::Proxy { evict_values: true })
            && self.db.get(keyset_key(key).as_slice()).is_some()
        {
            return Ok(None);
        }
        Ok(self.db.get(key).map(|b| b.to_vec()))
    }

    fn range_user(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if matches!(self.role, FoldRole::Relay) {
            return Ok(Vec::new());
        }
        let start = prefix.to_vec();
        let end_owned = prefix_exclusive_end(prefix);
        let rows = self.db.range(
            std::ops::Bound::Included(start.as_slice()),
            match end_owned.as_deref() {
                Some(e) => std::ops::Bound::Excluded(e),
                None => std::ops::Bound::Unbounded,
            },
        );
        let mut out = Vec::new();
        for (k, v) in rows {
            // Only fold meta (`\0fold/cursor`, `\0fold/keyset/…`). User keys
            // may start with 0x00 (FDB tuples) — F67.
            if crate::follow::is_fold_meta_key(&k) {
                continue;
            }
            out.push((k.to_vec(), v.to_vec()));
        }
        Ok(out)
    }
}

impl FoldStore for PedraFold<StdEnv> {
    fn open(path: &Path) -> Result<(FoldCursor, Self)> {
        Self::open_role(path, FoldRole::Storage)
    }

    fn apply(&mut self, batch: &[FoldUpdate], cursor: FoldCursor) -> Result<()> {
        apply_inner(self, batch, cursor)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.get_user(key)
    }

    fn range(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.range_user(prefix)
    }

    fn cursor(&self) -> FoldCursor {
        self.cursor
    }
}

/// F97: `\\0fold/keyset/ || u32be(len) || user` so keyset(a) is not a
/// byte-prefix of keyset(ab) under half-open scans.
fn keyset_key(user: &[u8]) -> Vec<u8> {
    let mut k = KEYSET_PREFIX.to_vec();
    let n = u32::try_from(user.len()).expect("fold keyset user len fits u32");
    k.extend_from_slice(&n.to_be_bytes());
    k.extend_from_slice(user);
    k
}

#[cfg(test)]
mod keyset_tests {
    use super::*;

    #[test]
    fn keyset_key_not_prefix_of_sibling_user() {
        let a = keyset_key(b"a");
        let ab = keyset_key(b"ab");
        assert!(
            !ab.starts_with(&a),
            "keyset_key(a) must not prefix keyset_key(ab): {a:?} vs {ab:?}"
        );
        assert_ne!(a, ab);
        assert_ne!(keyset_key(b"a/b"), keyset_key(b"a"));
        assert_ne!(keyset_key(b"a\0b"), keyset_key(b"a"));
    }
}
