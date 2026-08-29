# Compat OOTB = rust-rocksdb factory

**Bar:** one `Options::default()` for any host that `use rocksdb`. Official
parity is always that factory (`WriteOptions.sync=false`), Linux and Mac.

| knob | before (Pedra-shaped) | now (Rocks factory) |
|---|---|---|
| `sync` | false | false (unchanged) |
| `write_buffer_size` | 4 MiB | **64 MiB** (`0x4000000`) |
| `wal_full_fsync` | true | **true** (CMake Rocks Darwin `HAVE_FULLFSYNC`; not the sys-crate hole) |
| blob | off | off |
| WAL recovery | PointInTime | PointInTime |

`set_sync(true)` on Mac is **CMake Rocks** `F_FULLFSYNC`. crates.io
`librocksdb-sys` 0.16 omits `HAVE_FULLFSYNC` — that is a binding
oversight, not the Rocks default. See
[`../2026-08-27-upstream-fullfsync`](../2026-08-27-upstream-fullfsync/README.md).

Test: `default_options_match_rust_rocksdb_factory`.
