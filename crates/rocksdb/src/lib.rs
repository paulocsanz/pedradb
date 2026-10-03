//! Crate-name shim: `rocksdb` 0.21 API subset on Pedra.
//!
//! Upper databases that embed RocksDB depend on the crates.io `rocksdb`
//! crate by name (SurrealDB `kv-rocksdb` requires `rocksdb = "0.21.0"`).
//! This package carries that exact name and version so a
//! `[patch.crates-io] rocksdb = { path = … }` line swaps the storage
//! engine underneath them without touching their sources (RFC-0059).
//!
//! Everything is [`rocksdb_compat`] verbatim except [`Transaction`]:
//! rust-rocksdb spells the second parameter as the DB type
//! (`Transaction<'a, OptimisticTransactionDB>`); Pedra's is the injected
//! `Env`. The projection keeps upper sources compiling unmodified,
//! including SurrealDB's `transmute` to `Transaction<'static, _>` — both
//! sides of that transmute resolve to the same
//! `rocksdb_compat::Transaction<'a, IoUringEnv>` with only the lifetime
//! differing, so the lifetime extension is the single safety obligation
//! (the transmuted handle must not outlive the `OptimisticTransactionDB`,
//! which SurrealDB guarantees by keeping `_db: Pin<Arc<…>>` alive in the
//! same struct).

pub use rocksdb_compat::{
    backup, checkpoint, BackupEngine, BackupEngineOptions, BottommostLevelCompaction, Checkpoint,
    ColumnFamilyDescriptor, CompactOptions, DBCompactionStyle, DBCompressionType,
    DBRawIteratorWithThreadMode, DBRecoveryMode, Env, Error, LogLevel, OptimisticTransactionDB,
    OptimisticTransactionOptions, Options, ReadOptions, RestoreOptions, SliceTransform,
    SnapshotWithThreadMode, SstFileManager, TransactionDB, TransactionDBOptions,
    TransactionOptions, UniversalCompactOptions, UniversalCompactionStopStyle,
    WaitForCompactOptions, WriteOptions, KNOB_INVENTORY,
};

/// Maps a rust-rocksdb DB type to the Pedra `Env` its handle owns. The
/// shim's [`Transaction`] alias projects through this so the DB parameter
/// is real, not cosmetic.
pub trait TransactionDb {
    /// Pedra `Env` backing this DB handle (bounds are checked where the
    /// alias is instantiated).
    type Env;
}

impl TransactionDb for OptimisticTransactionDB {
    type Env = pedradb_io_uring::IoUringEnv;
}

impl TransactionDb for TransactionDB {
    type Env = pedradb_io_uring::IoUringEnv;
}

/// rust-rocksdb `Transaction<'a, D>` (D = `OptimisticTransactionDB`).
pub type Transaction<'a, D> = rocksdb_compat::Transaction<'a, <D as TransactionDb>::Env>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_transaction_alias_is_compat_transaction() {
        // The alias must erase the DB parameter: SurrealDB names
        // `Transaction<'static, OptimisticTransactionDB>` while
        // `transaction_opt` returns the same concrete type.
        fn assert_same<'a>(
            _named: Option<Transaction<'a, OptimisticTransactionDB>>,
            _ret: Option<rocksdb_compat::Transaction<'a, pedradb_io_uring::IoUringEnv>>,
        ) {
        }
        assert_same(None, None);
    }
}
