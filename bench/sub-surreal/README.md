# sub-surreal — SurrealDB substitution bench (RFC-0059)

`surrealdb-core` v1.5.4 (upstream tag `v1.5.4`, vendored verbatim under
`surreal-core/`) compiled twice against the same kvs code, with the storage
crate swapped:

- `pedra/` — `[patch.crates-io] rocksdb = { path = "../../crates/rocksdb" }`
  (crate-name shim over `rocksdb-compat`); SurrealDB sources untouched.
- `peer/` — real `rocksdb` 0.21 from crates.io (bundled C++ RocksDB), the
  official parity peer (SurrealDB itself sets `WriteOptions::set_sync(false)`
  in its transaction path, matching `ROCKS_PARITY_SYNC=0`).

Both build the same driver (`sub-bench`), which drives SurrealDB kvs
`Datastore`/`Transaction` — not raw KV — through four legs: `point_read`
(zipf point gets in read txns), `point_write` (zipf sets in write txns),
`scan` (kvs `scan` = `raw_iterator_opt` + seek), `doc_txn` (RMW:
get-modify-set-commit, the OCC validation path). Read txns end with
`cancel()` — SurrealDB kvs `commit()` is write-only by contract.

## Run

```sh
cd bench/sub-surreal/pedra && cargo build --release
SUB_BENCH_ENGINE=pedra ./target/release/sub-bench
cd ../peer && cargo build --release   # first build compiles C++ RocksDB
SUB_BENCH_ENGINE=rocks ./target/release/sub-bench
```

Offline (VM gates behind a resolver that drops `index.crates.io`): run
`sh bench/sub-surreal/vendor.sh` once from the repo root — merges both
sides' crates.io deps into `vendor/` (gitignored) and writes the
`.cargo/config.toml` source replacement — then `cargo build --release
--offline` on either side.

Knobs: `SUB_BENCH_SECONDS` (leg duration, default 8), `SUB_BENCH_RECORDS`
(seed size, default 1024), `SUB_BENCH_DIR`, `SUB_BENCH_OPS` (informational).
Output: `SUB_BENCH_JSON {…}` single line — ratios computed outside.

No ratio floor gate here: this is the substitution proof (upper DB code,
storage swapped underneath) plus telemetry of where upper-layer cost lives.
