# PedraDB architecture

A clean-room reimplementation of RocksDB-style LSM-tree storage, in Rust.

## Why clean-room, not CXX translation

RocksDB is ~500k lines of C++ with deep, pervasive cross-module coupling and
heavy use of features CXX cannot cross (`std::map<K,V>` templates, arbitrary
virtual dispatch, exceptions). Translating it module-by-module through a
`cxx::bridge` would mean exposing hundreds of internal types in the bridge,
fighting CXX's fixed type set at every step, and producing a hybrid nobody can
maintain.

Instead we treat RocksDB's **design** as the spec and rebuild each layer in
idiomatic Rust. The real RocksDB (C++) is used only as an external test oracle
via the `pedradb-oracle` crate — never linked into the shipped engine.

> CXX is the right tool for the *oracle* (binding the public C API) and for any
> future opt-in interop, but not for translating the engine internals.

## The oracle strategy

Same workload, two engines, diff the resulting state:

```
        ┌──────────── ops (deterministic) ────────────┐
        │                                              │
        ▼                                              ▼
 ┌──────────────┐                              ┌──────────────┐
 │ pedradb-core │                              │  RocksDB C++ │  (oracle)
 │   (Rust)     │                              │  via oracle  │
 └──────┬───────┘                              └──────┬───────┘
        │                                             │
        └────────── diff snapshots ───────────────────┘
                       (must be equal)
```

A failing diff is a real bug in our Rust engine, not a test artifact. This is
the cross-validation discipline: a claim about correctness is only settled once
it agrees with the independent ground truth, not on a single self-test.

## Delivery slices

Each slice is independently testable and ships a usable artifact. Lower layers
are built first so upper layers have stable foundations.

| Slice | Status | What it delivers                          |
|-------|--------|-------------------------------------------|
| 1. WAL         | ✅ done | Append-only crash-safe log (block-based, masked CRC32C, First/Middle/Last fragmentation, fsync, recovery) |
| 2. MemTable    | ⏳ next | In-memory sorted buffer (skip-list or `BTreeMap`) over which writes are applied |
| 3. SST format  | 🔲 | Block-based sorted-string table reader/writer (data + index + filter blocks) |
| 4. Flush       | 🔲 | MemTable → immutable → SST file when it crosses a threshold |
| 5. Get / point lookup | 🔲 | MemTable ∪ SSTs, tombstones, bloom-filter short-circuit |
| 6. Compaction  | 🔲 | Size-tiered / levelled merge of SSTs |
| 7. Iterators   | 🔲 | Merged forward/reverse scans across all levels |
| 8. Transactions, snapshots, WAL recycling | 🔲 | Advanced features toward parity |

## WAL on-disk format (slice 1)

Mirrors RocksDB's `db/log_format.h` so the two are directly comparable and so a
PedraDB WAL is parseable by either engine.

- The log is a sequence of **32 KiB blocks**.
- Each **physical record** has a 7-byte header: `CRC32C(4) | length(2) | type(1)`,
  little-endian, followed by a payload fragment.
- The CRC is RocksDB-masked (`rotate_right_15 + 0xa282_ead8`) over `{type, payload}`.
- Record types: `Full` (1), `First` (2), `Middle` (3), `Last` (4). A logical
  record larger than a block's remaining capacity is split into `First` then
  zero or more `Middle` then a `Last`.
- `Zero` (type 0, length 0) marks block tail padding.

**Crash contract:** a record is durable only after `sync_all`/`sync_data`
returns. A trailing partial record left by a crash is silently skipped on
recovery (treated as clean end-of-log).

## Crate layout

```
pedradb/
├── crates/
│   ├── pedradb-core/    # the engine (pure Rust, forbids unsafe)
│   ├── pedradb-oracle/  # RocksDB bindings + diff harness (dev/test only)
│   └── pedradb-cli/     # `pedra` CLI
├── docs/
│   └── architecture.md  # this file
└── clippy.toml          # doc-valid-idents for proper nouns
```

`pedradb-core` is `#![forbid(unsafe_code)]`. The oracle's live RocksDB binding
is behind the `live-rocksdb` feature so CI without a C++ toolchain can run the
core tests.
