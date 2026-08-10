# PedraDB

A clean-room reimplementation of RocksDB-style LSM-tree storage, in Rust.

PedraDB rebuilds RocksDB's storage layers (WAL, MemTable, SSTable, flush,
compaction) from scratch in idiomatic Rust, using the real RocksDB only as an
external test oracle. No C++ is linked into the shipped engine.

## Status

| Slice         | Status |
|---------------|--------|
| WAL           | ✅ done |
| MemTable      | ⏳ next |
| SST format    | 🔲 |
| Flush         | 🔲 |
| Get / lookup  | 🔲 |
| Compaction    | 🔲 |
| Iterators     | 🔲 |
| Transactions  | 🔲 |

See [`docs/architecture.md`](docs/architecture.md) for the full design.

## Build & test

```sh
cargo test --workspace          # core tests (no C++ needed)
cargo clippy --workspace --all-targets
cargo run -p pedradb-cli -- wal /tmp/demo.log   # WAL demo: write, fsync, recover
```

The live RocksDB oracle is opt-in (requires a C++ toolchain):

```sh
cargo test -p pedradb-oracle --features live-rocksdb
```

## Layout

- `crates/pedradb-core` — the engine (`#![forbid(unsafe_code)]`)
- `crates/pedradb-oracle` — RocksDB bindings + diff harness (dev/test only)
- `crates/pedradb-cli` — the `pedra` CLI

License: Apache-2.0.
