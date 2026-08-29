# Darwin `sync=true` — CMake Rocks vs crates.io rust-rocksdb

**Not an invention.** Primary sources in this tree + CMake.

## Upstream C++ Rocks (the real default on Mac)

`librocksdb-sys-0.16.0+8.10.0/rocksdb/CMakeLists.txt`:

```
check_cxx_symbol_exists(F_FULLFSYNC "fcntl.h" HAVE_FULLFSYNC)
if(HAVE_FULLFSYNC)
  add_definitions(-DHAVE_FULLFSYNC)
endif()
```

On Darwin `F_FULLFSYNC` exists, so CMake **always** sets it.

`rocksdb/env/io_posix.cc` `PosixWritableFile::Sync` (what `WriteOptions.sync=true` uses):

```
#ifdef HAVE_FULLFSYNC
  fcntl(fd_, F_FULLFSYNC)
#else
  fdatasync(fd_)
#endif
```

Facebook Rocks PR #14590 (Apr 2026) documents the contract: **Sync() = fdatasync (Linux) / F_FULLFSYNC (macOS)**.

## crates.io rust-rocksdb 0.22 = hole in *build.rs*, not in Rocks

`librocksdb-sys-0.16.0+8.10.0/build.rs` Darwin arm:

```
} else if target.contains("darwin") {
    config.define("OS_MACOSX", None);
    config.define("ROCKSDB_PLATFORM_POSIX", None);
    config.define("ROCKSDB_LIB_IO_POSIX", None);
}
```

**No `HAVE_FULLFSYNC`.** So the C++ we link for `rocks-parity-bench` `rocksdb` engine does `fdatasync` on Mac. That is **not** what `brew install rocksdb` / CMake Rocks does.

Later rust-rocksdb forks (e.g. zaidoon1 `build.rs`) **hardcode** `cfg.define("HAVE_FULLFSYNC", None)` on macos/ios and say CMake detects it via `check_cxx_symbol_exists`. That is an explicit fix of this omission.

## Do they know? (checked 2026-08-26)

Three different "they":

| Who | Knows? | Evidence |
|---|---|---|
| **Facebook C++ Rocks** | yes | `CMakeLists.txt` `check_cxx_symbol_exists(F_FULLFSYNC "fcntl.h" HAVE_FULLFSYNC)` then `-DHAVE_FULLFSYNC`. `PosixWritableFile::Sync` = `fcntl(F_FULLFSYNC)` vs `#else fdatasync`. Issue #11035 (Mac 6× slower after 6.27.3 because HAVE_FULLFSYNC landed). PR #13276 `MACOS_IGNORE_FULLFSYNC=1` **tests only**, "DO NOT use in production". |
| **Official rust-rocksdb** (`crates.io` `rocksdb`) | **no evidence** | GitHub search `repo:rust-rocksdb/rust-rocksdb` for `HAVE_FULLFSYNC` / `F_FULLFSYNC` / `fullfsync`: **0 issues**. Hole still in: `librocksdb-sys` 0.16.0+8.10.0 (what 0.22 links), **0.19.0+11.8.1** (what 0.25.0 2026-08-16 links), and **master `build.rs`**. Darwin arm is still only `OS_MACOSX` + POSIX. They compile C++ via `cc::Build` with a hardcoded define list — CMake never runs, so the CMake probe never fires, and nobody copied the result into the list. |
| **zaidoon1 fork** (`rust-rocksdb` crate, `rust-librocksdb-sys`) | yes, documented | CHANGELOG **0.49.0 (2026-05-18)**: *"fix: define `HAVE_FULLFSYNC` on Apple targets … so RocksDB takes the `fcntl(F_FULLFSYNC)` path for true on-disk durability rather than the weaker plain `fsync` (which on macOS only flushes to the drive cache). Matches RocksDB's CMakeLists.txt."* `build.rs` comments: *"RocksDB's CMakeLists.txt detects it via check_cxx_symbol_exists; we hardcode it for Apple targets since the constant is universally available."* That fix **did not** land on official rust-rocksdb. |

Nit on zaidoon's changelog: the C++ `#else` is `fdatasync`, not `fsync` (`PosixWritableFile::Sync`). `Fsync()` `#else` is `fsync`. Same hole either way.

Makefile path (`build_tools/build_detect_platform`) Darwin OS block only sets `OS_MACOSX`, **but later it compile-tests `fcntl(0, F_FULLFSYNC)` and adds `-DHAVE_FULLFSYNC`**. CMake does the same via `check_cxx_symbol_exists`. Official rust-rocksdb copies the Darwin OS defines and **drops the compile-test**. They don't run CMake, they don't run the Makefile probe — hardcoded list, `HAVE_FULLFSYNC` never entered it.

## What we bench

Two reconstructions, not equally honest:

| peer | what it pays | vs CMake `HAVE_FULLFSYNC` |
|---|---|---|
| `ROCKS_PARITY_FULL_SYNC=1` | Rocks `fdatasync` **then** `File::sync_all` on **every** `*.log` in the dir | extra `fdatasync` (~50 µs) + may FULLFSYNC recycled WALs, not only the live fd |
| `CXXFLAGS=-DHAVE_FULLFSYNC` on the `librocksdb-sys` compile | `cc` 1.0.79 forwards `CXXFLAGS`; `PosixWritableFile::Sync` compiles the `fcntl(F_FULLFSYNC)` arm | **CMake class**, one syscall on the live WAL fd |

This tree's release `librocksdb-sys` build had `CXXFLAGS = None` (see `target/release/build/librocksdb-sys-*/output`). Linked C++ is the hole.

Protocol for Darwin B (coluna, not cartaz):

```
CXXFLAGS=-DHAVE_FULLFSYNC cargo build -p rocksdb-parity-bench --release --features real
# then PEDRA_PARITY_G1=1  ROCKS_PARITY_SYNC=1  ROCKS_PARITY_FULL_SYNC=0
```

`FULL_SYNC=0` when the C++ already has the define — otherwise double `F_FULLFSYNC`.

Dirty 2026-08-27 `FULL_SYNC=1` numbers (`findings/2026-08-27-darwin-b-upstream-ff`): p50 tied, min 0.944. Not a 3/3 quiet gate. Next measure is the `CXXFLAGS` peer on a quiet box.

Official cartaz remains `sync=false` (coluna A). This file is only coluna B / `set_sync(true)` on Mac.

## Compat default

`Options::wal_full_fsync` default **true** — match CMake Rocks, not the sys crate.
`write_buffer_size` default **64 MiB** — C++ `0x4000000`.
