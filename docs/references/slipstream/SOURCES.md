# Slipstream primary sources (fetched 2026-08-14)

Repo: https://github.com/beyondoss/slipstream
Crate: `beyond-slipstream` **0.7.2** (README still advertises 0.5; trust Cargo.toml)
License: MIT
Rust: 1.92, edition 2024
Model checker: `stateright = 0.31.0` (dev-dep)

Local snapshots in this directory:

| File | What |
|------|------|
| `README.md` | Product contract, API, snapshot format |
| `ARCHITECTURE.md` | Full protocol: watch_applied, floor guard, export/import, failure table |
| `BACKENDS.md` | fjall vs RocksDB numbers at 500M routes |
| `Cargo.toml` | Features + deps |
| `protocol.rs` | The three shared kernels (prod == model) |

## Source tree (main)

```
src/
  applied.rs          watch_applied combinator
  artifact.rs         export/import staging
  export_lease.rs     CAS lease, embedded TTL
  kv.rs               traits
  lib.rs
  nats.rs             JetStream mapping + floor guard
  protocol.rs         pointer_publish_allowed, payload_prunable, resume_window_ok
  snapshot.rs         AppendLogSnapshot (PGSS + CRC)
  snapshot_fjall.rs
  snapshot_record.rs
  snapshot_rocksdb.rs
  stores.rs
  transport.rs        object_store pointer-swap
tests/
  model.rs            Stateright theorems on the same kernels
  model_applied.rs
  model_live_watch.rs
  resync.rs           nats_silently_clamps_resume_below_first_seq
benches/
  snapshot.rs, ack.rs, applied.rs, snapshot_backends.rs
```

## What this crate is (from their words)

> “You have config in NATS JetStream: routing tables, TLS certs, WASM configs.
> Edge nodes need a local copy, kept in sync, that survives restarts without
> replaying the full stream.”

> “NATS is a bounded log. … Once retention compacts past a cursor, there is
> no replay path from NATS. The local fold is the durable state; folds across
> the fleet are the only full replicas.”

It is **not** a transactional database. It is a **fold** of a last-write-wins
KV change stream, plus a bootstrap path when the stream has forgotten you.

## Invariants they encode once (do not re-derive)

1. **Cursor-after-apply.** A persisted cursor `C` means every update with
   revision ≤ `C` has had `apply()` return. Receipt is the wrong signal
   (Saltzer/Reed/Clark end-to-end argument; Consul “reconciled index”).
2. **Atomic `apply(batch, cursor)`.** Data and cursor advance together.
   A torn write may leave data *ahead* of the cursor (replay is safe),
   never a cursor naming missing data.
3. **Transient store failure re-queues.** Advancing the cursor over a failed
   `store.apply` leaves a hole that survives every restart
   (`transient_store_failure_never_leaves_a_cursor_gap`).
4. **`resume_window_ok(rev, first_seq)`.** NATS *silently clamps* a below-head
   start. They refuse and take the `CursorExpired` → resync path.
5. **Live floor guard (All-scope only).** In-band: delivery that jumps the
   frontier by >1 is checked against `first_sequence` *before* apply.
   30 s backstop for the no-traffic case. Prefix watches cannot do this
   (sparse revisions look like gaps).
6. **Cursor-expired resync.** Re-list cannot resurrect deletes whose markers
   were evicted. Diff fold keys vs live keys → synthetic `Delete` (unknown
   version, does not advance cursor) → *then* state-sync re-list.
7. **State-sync watches.** Non-`_from` watches are `LastPerSubject` (current
   value of every key, then live). Seed-then-watch has a race.
8. **Export pointer is monotone.** Content-addressed payload
   `blake3(manifest)[..8].tar` + CAS pointer. `pointer_publish_allowed`
   refuses strictly older. `payload_prunable` is strictly-below (age-only
   prune produced a dangling pointer in the model).
9. **Lease is dedup, not correctness.** Clock skew ⇒ duplicate artifact,
   last-write-wins. Corrupt lease/pointer is stealable, not a wedge.
10. **Snapshot is a cache** while the log still holds the tail. After the
    log evicts past every cursor, **folds are the only full replicas**.

## Backend numbers they published (500M routes, ~60 B keys, ~200 B values)

| | fjall | rocksdb |
|--|-------|---------|
| Hydrate 500M | 1475 s (0.34 M/s) | 2552 s (0.20 M/s) |
| `settle()` | 1127 s, peak 203 GiB (2×) | 41 s, 105 GiB |
| Cold get p50 / p999 | 542 µs / 3.7 ms | 292 µs / 898 µs |
| Absent-key p50 | 421 ns | 321 ns |

Unsettled trees: cold get 8–10× slower. `settle()` is mandatory before serve.

## Cross-link

Pedra already treats CHANGELOG as a cache and WAL as SoR
(`crates/pedradb-core/src/db_kernel.rs`, RFC-0019). Slipstream is the **consumer-side**
discipline for that seam, plus the “log compacted, now what” story.
See [`../../nats-need-replacement.md`](../../nats-need-replacement.md) for
why JetStream is not a durability oracle.
