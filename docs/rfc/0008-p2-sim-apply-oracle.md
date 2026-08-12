# RFC-0008: P2 sim, apply_batch, oracle, research-opts deferral

**Status:** done (P2.1 base); **Env-level faults extended in [RFC-0011](0011-env-fault-injection.md)**  
**Updated:** 2026-08-11  

| Slice | Status | Delivery |
|-------|--------|----------|
| P2.1 | done | `pedradb-sim`: FaultEnv, crash-after-sync, WAL truncate tail |
| P2.1b | → RFC-0011 | `Env` seam + `FailingEnv` (fail-after-N, FaultKind) — P0 shipped |
| P2.2 | n/a | Deferred until measured; baseline `benches/baseline.rs` |
| P2.3 | done | `Db::apply_batch`, `Snapshot`, `get_at` |
| P2.4 | done | ModelStore default; `live.rs` RocksDB under `--features live-rocksdb` |
