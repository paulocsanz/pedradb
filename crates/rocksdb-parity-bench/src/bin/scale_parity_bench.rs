//! Sorted-ingest scale bench: Pedra always; RocksDB behind `--features real`;
//! Fjall behind `--features fjall`.
//!
//! ```text
//! SCALE_ENTRIES=1000000 cargo run --release -p rocksdb-parity-bench --bin scale-parity-bench -- /tmp/scale-pedra
//! SCALE_BACKENDS=fjall SCALE_ENTRIES=1000000 cargo run --release -p rocksdb-parity-bench --features fjall --bin scale-parity-bench -- /tmp/scale-fjall
//! SCALE_BACKENDS=rocksdb SCALE_ENTRIES=1000000 cargo run --release -p rocksdb-parity-bench --features real --bin scale-parity-bench -- /tmp/scale-rocks
//! ```
//!
//! 25M/100M: one backend per process (default). See `scripts/reproduce-scale.sh`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "findings/scale-parity".into());
    rocksdb_parity_bench::scale::run(&PathBuf::from(out));
}
