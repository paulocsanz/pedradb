# PedraDB / Montanha audit: `unsafe` and FFI

**Scope:** workspace `unsafe` tokens, `forbid`/`deny` map, POSIX / Linux / Apple / Windows I/O FFI, C ABI (`c-api`), adjacent C++ oracle FFI.
**Date:** 2026-08-22
**Collector:** `.grok/skills/audit-pedradb/collect-audit-data.sh` (unsafe section) + full `rg` of `crates/**/*.rs`
**Tests run this session:** P0/P1 as above; P2: `cargo test -p pedradb-capi` (11), `cargo check -p pedradb-store --lib`, `bash scripts/unsafe-surface.sh`. No Miri, no Linux io_uring soak.

This is **not** a durability/WAL audit. Durability is mentioned only where an `unsafe`/FFI island can break G1 (`fdatasync` before Ok).

---

## Overall score: 8/10 (unsafe/FFI discipline)

The kernel is already the thing we wanted: `pedradb-core` is `#![forbid(unsafe_code)]`, production writes go through one tiny POSIX island, io_uring and the C ABI are opt-in and crate-isolated. The remaining work is not “stop using unsafe” — it is (1) close one incomplete io_uring CQE-tag fix, (2) treat the C ABI as a real FFI product or move it out of `pedradb-store`, (3) give each island a scrutinizable safe surface + SAFETY notes.

---

## 1. Inventory

### 1.1 Real `unsafe` in this repo (product + lab)

| Crate | Kind | Sites | Default in product path? |
|-------|------|-------|--------------------------|
| `pedradb-posix` | POSIX FFI `fdatasync(2)` | 1 `unsafe extern "C"` block + 1 call | **Yes** — `StdEnv` / WAL G1 on Unix |
| `pedradb-io-uring` | Linux `io_uring` SQE push (`ring.rs`) | `submit_sqe` | **Yes** — production `open` (POSIX fallback off-Linux / ring-setup fail) |
| `pedradb-capi` | C ABI marshalling (handle table) | `unsafe extern "C"` + slice/`CStr` | **No** — lab `cdylib`, store does not link it |

False positives (prose, not Rust `unsafe`):

- `pedradb-core/src/db.rs` — “post-commit mismatch is unsafe”
- `pedradb-raft/src/ae_kernel.rs` — log line `"F16 unsafe at idx=…"`

**Count:** 20 `unsafe` token lines in `crates/`; ~14 are real operations. Core, raft, SQL, HTTP, fold, DCS, sim, DST, compat, oracle *source* have zero.

### 1.2 `forbid` / `deny` map

| Policy | Crates |
|--------|--------|
| `#![forbid(unsafe_code)]` | `pedradb-core`, `pedradb-store` (C ABI extracted), apply, dcs, raft, http, sql, stream, replicate, journal, lease, index, fold, ops, sim, dst, oracle, rocksdb-compat, rocksdb-parity-bench, cli, alias-smoke, `montanha-fdb-recipes`, plus store kernels/bins |
| **No crate-level forbid** | `pedradb-posix`, `pedradb-io-uring`, `pedradb-capi` (these *are* the `unsafe` islands) |

### 1.3 Adjacent FFI (not our `unsafe`, still TCB if you turn the feature on)

| Path | What it links | Linked into core? |
|------|----------------|-------------------|
| `pedradb-oracle` feature `live-rocksdb` | crates.io `rocksdb` 0.22 → `librocksdb-sys` (C++ RocksDB + bindgen) | **No** (oracle-only; crate itself is `forbid`) |
| `rocksdb-parity-bench` feature `real` | same | **No** (bench process only) |
| `io-uring` 0.7 (Linux dep of `pedradb-io-uring`) | crate-internal `unsafe` + `libc` | Only if you construct `IoUringEnv` |

Doctrine already forbids linking Rocks into the engine. Keep it that way. The C++ surface is **oracle/bench TCB**, not Pedra TCB.

---

## 2. Data flow: where unsafe sits relative to the kernel

```
  App / CLI / HTTP / Store / C ABI
           │
           ▼
  pedradb-core  (forbid unsafe)
      Env::sync_data / write / advise
           │
     ┌─────┴──────────────────────┐
     ▼                            ▼
 StdEnv                      IoUringEnv (opt-in)
     │                            │
     ▼                            ├─ Linux: io_uring SQE  (unsafe here)
 pedradb-posix                    └─ else: StdEnv / fdatasync_file
 fdatasync(2)  (unsafe here)
```

`Db::open` and the CLI use `StdEnv`. Linux io_uring is an explicit `pedradb_io_uring::open` / `IoUringEnv`. The C ABI wraps `StoreCluster` and never touches the WAL FFI itself.

That split is the main good news: you can audit POSIX durability, io_uring, and C marshalling as three small crates without reading the LSM.

---

## 3. Island A — `pedradb-posix` (production TCB)

**Safe surface today:** `fdatasync_file(&File) -> io::Result<()>`

**Unsafe:**

```rust
unsafe extern "C" {
    fn fdatasync(fd: i32) -> i32;
}
let rc = unsafe { fdatasync(file.as_raw_fd()) };
```

### Why it exists

On Apple, `std::fs::File::sync_data` / `sync_all` are `fcntl(F_FULLFSYNC)` (~5 ms on the lab Mac). RocksDB / TiKV `WriteOptions.sync` call libc `fdatasync` (~30–50 µs). RFC-0036 moved WAL G1 onto the **same barrier class** as that peer. `libc` / rustix **omit** `fdatasync` on Apple (they want `F_FULLFSYNC`), so the crate declares the POSIX symbol itself.

Core stays `forbid` by depending on this crate (`env::fdatasync_file` is a one-line wrapper).

### Safety obligations (must hold)

1. `file` is an open, live `std::fs::File` for the call. `as_raw_fd()` is not retained after return.
2. The linked `fdatasync` is the POSIX/libSystem symbol: `int fdatasync(int fd)`, C ABI, `fd` is a valid descriptor.
3. `rc == 0` is success; otherwise `Error::last_os_error()` (errno from this thread).
4. Non-Unix: no FFI; `File::sync_data()` (Windows: `FlushFileBuffers`).

There is **no `SAFETY:` comment** on the block. The crate has no `SAFETY.md`.

### Platform semantics (this is the product risk, not UB)

| Host | What G1 actually is | Power-loss vs `F_FULLFSYNC` |
|------|---------------------|----------------------------|
| Linux | `fdatasync(2)` on the WAL fd | POSIX data-sync; device write-cache policy is the disk’s |
| macOS / Darwin | libSystem `fdatasync` — **not** `F_FULLFSYNC` | **Weaker than Rust std.** Drive cache can hold “synced” WAL across a power cut. Same class as Rocks/TiKV `fdatasync`, **intentionally**. |
| Windows | `File::sync_data` → `FlushFileBuffers` | Typically stronger than Darwin `fdatasync`; no Pedra FFI |
| Other Unix | same symbol as Linux if libc exports it | Untested (no CI matrix in this audit) |
| `wasm` / non-unix | `sync_data` | n/a |

**Directory fds:** `StdEnv::sync_dir` calls `fdatasync_file` on a directory `File`. POSIX `fdatasync` is specified for regular-file data. Linux treats `fdatasync(dirfd)` like a metadata sync in practice; Darwin is less documented. If directory publish (SST rename + `CURRENT`) must be power-loss durable on Apple at `F_FULLFSYNC` strength, this path does **not** do that. That is a **contract** issue, not a Rust-UB issue — but it lives in this FFI island.

**EINTR:** a failed `fdatasync` is propagated as `Err`. Kernel may or may not have completed the barrier (same “uncertain outcome” as RFC-0015 H1). Not unique to unsafe.

**MSRV:** workspace `rust-version = "1.75"`. `unsafe extern` blocks stabilized in **1.82**. Current lab `rustc` is 1.97, so it builds here; a 1.75 builder would fail. Either bump MSRV or write a plain `extern "C"` (call remains `unsafe`).

### Residual in this island

- Missing SAFETY comment / crate SAFETY.md
- No Miri job (syscall FFI; Miri will not emulate `fdatasync` usefully — residual is honest)
- Darwin directory-fd + power-loss semantics not characterized in tests
- 32-bit / musl / Android not in the audit matrix

---

## 4. Island B — `pedradb-io-uring` (opt-in Linux TCB)

**Safe surface today:** `IoUringEnv` implements `Env` / `EnvFile`. Callers never write `unsafe`. Backend is `IoUring` or `PosixFallback`.

**Unsafe (Linux only):**

| Site | Op | Stated invariant |
|------|----|------------------|
| `uring_write` | `submission().push` of `Write` | `buf` valid until `submit_and_wait` returns |
| `uring_fsync` | `Fsync` / `DATASYNC` | no buffer; wait before return |
| `sync_dir` | `Fsync` on dir fd | dir `File` lives until wait returns |
| `advise` | `libc::posix_fadvise` | fd open; offset/len best-effort |

Reads still go through `std::fs`. Only write + fsync + dir fsync use the ring. One `IoUring` (entries=64) behind `parking_lot::Mutex`, shared by all files of that env. SQ depth 64 with wait-per-op ⇒ at most one *logical* in-flight op per env.

### What is already good

- Isolated crate; core `forbid` untouched.
- POSIX fallback on macOS / ring-setup failure (no panic).
- F202: `Seek` uses the shadow cursor, not the kernel O_APPEND offset (WAL reopen corruption).
- F203: `wait_tagged_cqe` matches `user_data` instead of taking `completion().next()` blindly — **cross-opcode** stale CQEs are dropped.

### HIGH — F203 was incomplete for same-opcode reuse (fixed 2026-08-22)

**Fixed:** unique `next_user_data` per SQE + harvest matching CQE even when `submit_and_wait` returns Err (`cqe_kernel.rs` / `submit_sqe`). Kernel tests cover as-is constant tags vs unique tags on every host. The Linux ring path is still NEEDS-LINUX-ENV.

Tags **were** constants: write `0x77`, fsync `0x5f`, dir `0xd1`. Sequence:

1. `push(SQE tag=0x77)` succeeds.
2. `submit_and_wait` returns `Err` (e.g. `EINTR`) **after** the kernel accepted the SQE.
3. The function `?`-returns. `self.pos` is not advanced. CQE `0x77` sits (or will sit) in the CQ.
4. Next `uring_write` pushes **another** `0x77`, waits, `wait_tagged_cqe` takes the **stale** `0x77` as the new op’s result.

Effects:

- Wrong `res` / wrong cursor advance (short write reported as full, or vice versa).
- **False `Ok` on `fsync`:** a stale successful `0x5f` makes the next `sync_data` return Ok without the **new** fsync completing. On `IoUringEnv` that is a G1 hole (ack without *this* barrier).

F203 only drains CQEs whose tag **differs** from the current op. Same-tag leftovers are treated as the current op.

This is **HIGH** (opt-in Env, needs submit-error-after-issue). It becomes a production G1 bug **if** Linux production defaults to `IoUringEnv`. Today the CLI / `Db::open` do not.

**Fix shape (do not “fix” by ignoring EINTR):**

1. Monotonic unique `user_data` per SQE.
2. On `submit_and_wait` error, still harvest **this** tag (or cancel) before returning, so the next op never sees a leftover.
3. Regression: inject submit `EINTR` after push (needs a test double or Linux fault); k-style test if a seam exists.

### Other issues (MEDIUM)

| Item | Risk |
|------|------|
| `buf.len() as u32` | Writes >4 GiB truncate the SQE length; `Write` short-write contract, not UB. RFC-0048 already treated this as a dead end. |
| `io_uring_supported() -> true` on all Linux | Lies if `IoUring::new` failed; callers should use `backend()`. |
| `posix_fadvise` lives here | `StdEnv::advise` is a no-op even on Linux. Prefetch (RFC-0029) only works if you opted into this crate. The unsafe belongs in `pedradb-posix`. |
| `io-uring` 0.7 TCB | All ring `unsafe` in that crate is inherited. `cargo deny` advisories cover RUSTSEC; there is no geiger/Miri gate that actually runs. |
| SAFETY comments | Present but incomplete (fd liveness, exclusive SQ via mutex, unique tags). |
| No registered `cdylib`/Miri in CI | Documented residual (`docs/synthetic-field-residuals.md` P2.5). Miri cannot talk to a real ring. |

### Concurrency

`Mutex` around the ring serializes SQ/CQ. `IoUringFile` is `Send` if the env is. Two threads writing two files of the same env queue on that mutex (correct, slow). There is no `unsafe impl Send`. Do not add SQPOLL / IOPOLL without a new audit.

---

## 5. Island C — C ABI `montanha_fdb_*` (lab TCB)

**Files:** `crates/pedradb-capi/` (`handles.rs` + `lib.rs`), `include/montanha_fdb.h`
**Crate:** `pedradb-capi` (RFC-0023 P2.2). Header says “plug/test only”, “NOT the FoundationDB client”. **Fixed 2026-08-23:** slot+generation handles, not `Box::into_raw`.

**Safe logic already exists:** `StoreCluster` + `client::Transaction` + `fdb_compat`. The `unsafe` is only pointer marshalling.

### ABI

Opaque heap handles (`Box::into_raw` / `from_raw`), C strings, borrowed key/value slices, get-buffer allocated in Rust and freed with `montanha_fdb_free(p, len)`.

There is **no `crate-type = ["cdylib"]`**. A C consumer cannot link a shared library from the current `Cargo.toml`. The face is a Rust-callable C ABI for in-process tests, not a shipped `.so`/`.dylib`.

### HIGH — handle lifetime is raw `Box`, not a table

| Bug class | How |
|-----------|-----|
| Double-free | `commit` always `Box::from_raw(tr)` (“do not destroy after”). Destroy-after-commit or destroy twice is UB. |
| Use-after-free | Destroy `db` while a `tr` is live; or keep `tr` after commit. `Transaction` does not hold a pointer to the cluster, but `get`/`commit` take `db` again — a dangling `db` is UB. |
| Cross-handle mix-up | `get`/`commit` take both `db` and `tr` with **no** check they belong together. Two valid pointers to *different* clusters is not memory-UB; it is silent wrong OCC. |
| `free` length mismatch | `Box::from_raw(from_raw_parts_mut(p, len))` with the wrong `len` is allocator UB. |
| Threading | `StoreCluster` is a plain struct (HashMaps, nodes, generation). C callers can pass the same `db*` to two threads. That is a data race = UB. Nothing in the header says “not thread-safe”. |
| `CStr::from_ptr(path)` | Path must be NUL-terminated. Missing NUL → read off end. No max length. |

Null checks on entry are present (good). `elect_all` failure drops the unboxed cluster (good). **Update 2026-08-23:** C+ASan harness (`scripts/capi-asan.sh`) links a C TU; Rust tests alone are not the C-caller oracle.

### MEDIUM

- Error on `get` does not write `out_ptr`/`out_len` — C caller may read uninitialized outputs.
- Empty value and missing key are both `{null, 0}`.
- Failures return `NULL` / `MONTAHA_FDB_ERROR` with no errno/detail.
- Typo `MONTAHA` (missing ‘n’) is consistent in header + Rust; renaming is ABI.
- `pedradb-store` cannot `forbid(unsafe_code)` while this module lives here — the *store* crate is the wrong audit boundary.

---

## 6. Platforms — FFI risk summary

| Platform | Default engine I/O | Our FFI | UB risk | Durability / semantics risk |
|----------|--------------------|---------|---------|-----------------------------|
| **Linux** | `StdEnv` → `fdatasync(2)` | posix symbol | Low (fd + signature) | POSIX fdatasync class |
| **Linux + `IoUringEnv`** | `IORING_OP_WRITE` / `FSYNC` | `io-uring` + `libc::posix_fadvise` | Medium (SQE/CQE, buffer lifetime) | **HIGH** G1 hole if stale same-tag CQE after submit error |
| **macOS** | `StdEnv` → libSystem `fdatasync` | posix symbol | Low | **Intentional:** not `F_FULLFSYNC`. Power loss ≠ Rust `sync_data`. Dirfd sync class unclear |
| **Windows** | `FlushFileBuffers` | none | None in Pedra | Different class than Unix fdatasync; untested as a product OS |
| **C consumer** | N/A | `fdb_c` | HIGH if used as a real ABI (UAF/races) | Lab-only today |
| **Oracle/bench `rocksdb` feature** | C++ Rocks | bindgen/librocksdb-sys | Huge TCB, **out of product** | Must never become a core dep |

No `mmap`, no `unsafe impl Send/Sync`, no `transmute`, no `MaybeUninit` in engine code.

---

## 7. Findings

### Critical

None on the **default** `StdEnv` path. Core remains `forbid(unsafe_code)`.

### High

| ID | Location | Pattern | Issue | Fix |
|----|----------|---------|-------|-----|
| U1 | `pedradb-io-uring` constant `user_data` | Stale CQE adopted as current op | **Fixed 2026-08-22** unique tags; **soak 2026-08-23:** live ring in privileged Docker + CI `io-uring-linux-soak` (`linux_ring_is_live`, write/fsync/reopen, EINTR storm). | — |
| U2 | `pedradb-store` `fdb_c` | Raw `Box` handles | **Fixed 2026-08-23:** `pedradb-capi` slot+generation table. Stale/double-free → ERROR. **F215:** path/key/value lengths capped before copy; C+ASan harness (`scripts/capi-asan.sh`) PASS + malicious expected-FAIL. Slices under the cap remain C-contract `unsafe`. **Not product.** |

### Medium

| ID | Location | Issue | Fix |
|----|----------|-------|-----|
| U3 | `pedradb-posix` | `unsafe extern` vs MSRV 1.75 | **Fixed 2026-08-23:** edition-2021 `extern "C"` + crate `SAFETY.md`. |
| U4 | `StdEnv::sync_dir` | `fdatasync` on directory fds, Darwin semantics unknown | **Measured 2026-08-23:** file `fdatasync` p50 ~24 µs vs `F_FULLFSYNC` ~4 ms. Dirfd `fdatasync` **and** dir `sync_all` ~300 ns — `sync_all` on a Darwin dirfd is not file FULLFSYNC. Drive-cache plug-pull unsimulated (file-fdatasync class). |
| U5 | `posix_fadvise` in io-uring crate | Default Linux `StdEnv` never prefetches | **Fixed 2026-08-23:** `pedradb-posix::advise_file`; `StdEnv` + `IoUringEnv` call it. |
| U6 | `c-api` not a `cdylib` | Header over-promises linkability | **Fixed 2026-08-23:** `pedradb-capi` `cdylib` + `staticlib` + `rlib`. |
| U7 | `pedradb-cli`, alias-smoke | No `forbid(unsafe_code)` | **Fixed 2026-08-22:** crate-level `forbid`. |
| U8 | Miri / geiger | Residual documented, not gated | **Gated 2026-08-23:** allowlist `unsafe-surface.sh`; Miri `miri-unsafe-islands.sh`. Geiger report-only. |

### What’s good

- Core / raft / store kernels / HTTP / SQL stay `forbid`.
- POSIX and io_uring already live **outside** the LSM — the split RFC-0036 / RFC-0029 asked for is real.
- C ABI is a separate `pedradb-capi` crate; store is `forbid`.
- Rocks C++ is oracle/bench only.
- Hunt already closed F202 (cursor) and the *cross-tag* half of F203.
- `fdatasync_file` is a one-symbol safe API; that is the right shape to grow.

### Residual walls (honest)

- No Linux runner in this session: U1 is a **code-read** finding, not a red test.
- Darwin power-loss vs `fdatasync` is a hardware/OS fact, not something Rust `unsafe` can paper over.
- `io-uring` 0.7 remains unaudited third-party TCB (`libc` no longer a dep of `pedradb-io-uring`).
- Miri does not replace a kernel io_uring soak.

---

## 8. What to extract: libraries with a safe surface

Goal: every `unsafe` token lives in a crate whose **public API is 100% safe**, small enough to audit in one sitting, with Pedra/Montanha depending only on that API. Core, store (minus C), and products keep `forbid`.

Three islands already exist. The work is to **finish** the surfaces and (for C) **move** the module.

### 8.1 `pedradb-posix` — POSIX durability crate (keep, expand)

This is already the right library. Scrutinize it as if it were published.

**Safe API (shipped 2026-08-23):**

```rust
pub fn fdatasync_file(file: &File) -> io::Result<()>;
pub fn fsync_file(file: &File) -> io::Result<()>;
pub fn sync_dir_fd(dir: &File) -> io::Result<()>;
pub fn advise_file(file: &File, offset: u64, len: u64, kind: FileAdvise) -> io::Result<()>;
```

`FileAdvise` lives in this crate (not `AdviseKind`) so posix does not depend on core.

**Rules:**

- All `unsafe` / `extern "C"` / `libc::posix_fadvise` stay in this crate.
- `StdEnv::sync_data`, `StdEnv::sync_dir`, `StdEnv::advise`, and `IoUringEnv` POSIX fallback **only** call these functions.
- Crate-level `SAFETY.md`: fd liveness, symbol signature, Darwin ≠ `F_FULLFSYNC`, dirfd policy, EINTR/uncertain.
- Tests: happy-path fdatasync (exists), plus document that power-loss is not unit-testable here.
- Optional later: `fallocate` / `pread` / `pwrite` if the engine wants them without opening a fourth unsafe crate.

**Do not** put this back in `pedradb-core`. RFC-0036 already tried rustix-in-core and hit the Apple omission.

### 8.2 `pedradb-io-uring` — split inner ring from Env

Keep the crate (Linux-only dep on `io-uring`). Split **modules**:

```text
pedradb-io-uring
  src/ring.rs     // THE audit target: unsafe, unique tags, harvest-on-error
  src/env.rs      // safe Env impl; zero unsafe
  src/lib.rs      // re-export IoUringEnv
```

**Safe inner surface:**

```rust
pub struct UringQueue { /* Mutex<IoUring> + next_tag: AtomicU64 */ }

impl UringQueue {
    pub fn pwrite(&self, file: &File, buf: &[u8], offset: u64) -> io::Result<usize>;
    pub fn fsync(&self, file: &File, datasync: bool) -> io::Result<()>;
    pub fn fsync_dir(&self, dir: &File) -> io::Result<()>;
}
```

Invariants that belong **only** in `ring.rs`:

- `buf` borrowed until the matching CQE is harvested.
- fd borrowed from a live `File` for that wait.
- mutex held across push + submit + harvest (or a documented single-owner ring).
- unique `user_data`; leftover CQEs never match a future op.
- submit error still harvests (or cancels) the issued SQE.

`IoUringFile::write` / `sync_data` become safe wrappers around `UringQueue`. `advise` moves to `pedradb-posix`.

Whether `UringQueue` is a **separate published crate** is optional. The audit boundary is the module. Publishing is worth it if you want geiger/Miri/reviewers who should not read Pedra.

### 8.3 `pedradb-capi` (new) — C ABI off `pedradb-store`

Move `fdb_c.rs` + `include/montanha_fdb.h` out of the store crate. Store goes back to `#![forbid(unsafe_code)]`.

**Safe Rust side (already almost `fdb_compat`):**

```rust
pub struct Database { cluster: StoreCluster }
impl Database {
    pub fn open(path: &Path, n_nodes: u64, n_ranges: u64) -> Result<Self, StoreError>;
    pub fn begin(&mut self) -> Transaction;
}
```

C functions **only** marshal: CStr → `&str`, `*const u8`+len → `&[u8]`, handle → `&mut Database`.

**Do not export raw `Box` pointers.** Use a table:

```text
handle = (slot: u32, generation: u32)
table[slot] = Some({ generation, Box<T> })
destroy / commit: generation mismatch → error code, not free
```

Then: double-free and stale pointers become `MONTAHA_FDB_ERROR` instead of UB. Still require “valid UTF-8 C string / readable slice for `len`” (inherent to C). Document **not thread-safe** unless you put a mutex in `Database` (store is not a concurrent C API today).

`Cargo.toml`:

```toml
[lib]
crate-type = ["rlib", "cdylib", "staticlib"]
```

Tests: keep the Rust smoke tests; add one C harness under ASan **if** this becomes a product face. Until then the header should say “not a supported ABI”.

### 8.4 What not to extract / not to add

| Idea | Verdict |
|------|---------|
| `mmap` SST/WAL in core | Do not. Huge `unsafe` TCB; fjall-style forbid is the product. |
| rust-rocksdb as a runtime backend | Never. Oracle/bench only. |
| Merge posix + io-uring into one “sys” crate | Worse to audit (Apple + Linux ring in one TCB). |
| `unsafe` in memtable/WAL for speed | Needs a new RFC and a SAFETY budget (X6). Not this audit. |
| SQPOLL / IOPOLL / registered buffers | New io_uring audit; current mutex+wait model is the simple one. |

### 8.5 Suggested crate graph after extraction

```
pedradb-posix          ← only Unix/Windows durability FFI (scrutinize #1)
    ▲
pedradb-core (forbid) ─ StdEnv
    ▲
pedradb-io-uring       ← ring.rs unsafe; Env safe; depends on posix + core
    ▲
pedradb-store (forbid) ← no C
    ▲
pedradb-capi           ← only C marshalling + handle table (scrutinize #3)
```

Reviewers can read **~60 lines of posix**, **~150 lines of ring**, **~200 lines of capi** without opening `db.rs`.

---

## 9. Recommended delivery (if we implement)

**P0** — done 2026-08-22 (same session as the follow-up `vai`):

- [x] Unique io_uring tags + harvest on submit error (U1) — `cqe_kernel.rs` + `submit_sqe`. Kernel tests run on macOS (as-is constant tags take leftover; unique tags discard). Linux ring still NEEDS-LINUX-ENV.
- [x] `#![forbid(unsafe_code)]` on `pedradb-cli` and alias-smoke (U7).
- [x] `SAFETY:` comments on the posix `fdatasync` call/extern and each io_uring push (`submit_sqe` + `posix_fadvise`).

**P1** — done 2026-08-23:

- [x] `posix_fadvise` + `sync_dir_fd` + `fsync_file` in `pedradb-posix`; `StdEnv::advise` on Linux (U5). `IoUringEnv` dropped `libc`.
- [x] `ring.rs` module split; crate `SAFETY.md` for posix and io-uring.
- [x] MSRV vs `unsafe extern` (U3): plain edition-2021 `extern "C"` so `rust-version = "1.75"` still builds. Signature assertion stays in the SAFETY comment.

**P2** — done 2026-08-23:

- [x] Extract `pedradb-capi` with handle table; `pedradb-store` is `forbid(unsafe_code)` (U2, U6). `crate-type = ["rlib", "cdylib", "staticlib"]`. Double-destroy / stale handle tests.
- [x] `scripts/unsafe-surface.sh` allowlist gate (posix / io-uring / capi) in supply-chain CI. `cargo geiger` report-only if installed (`io-uring` 0.7 dep TCB).
- [x] Darwin dirfd / power-loss note in `docs/usage.md` (U4).

**P3** — done 2026-08-23:

- [x] Miri on the islands that can run: posix FFI (`fdatasync` 4/4 with isolation off), `cqe_kernel` 8/8, capi `handles` 5/5. Script `scripts/miri-unsafe-islands.sh`; CI `MIRI_REQUIRED=1`. Ring syscalls and capi `StoreCluster` tests remain residual.
- [x] `montanha-fdb-recipes` `#![forbid(unsafe_code)]` (was `deny`).

**P4** — done 2026-08-23 (environment residuals):

- [x] Linux live-ring soak: `linux_ring_is_live` + `linux_ring_soak_write_fsync_reopen` (256 mixed write/fsync, F202 cursor, 128 keyed put/flush/reopen/CRC). Privileged Docker Linux 17/17; CI `io-uring-linux-soak` (`URING_SOAK_REQUIRED=1`). Default Docker seccomp EPERM on `io_uring_setup` — script uses `--privileged`.
- [x] Darwin G1 class: `darwin_fdatasync_and_dirfd_are_not_fullfsync_class` — file `fdatasync` ~24 µs vs `F_FULLFSYNC` ~4 ms; dirfd both `fdatasync` and `sync_all` ~300 ns (not file FULLFSYNC). Drive-cache power-loss is the file-fdatasync class.

---

## 10. Layer / policy checks (this audit)

| Check | Result |
|-------|--------|
| `pedradb-core` `forbid(unsafe_code)` | pass |
| No `unsafe` in WAL/memtable/SST/TX | pass |
| Default I/O FFI isolated in `pedradb-posix` | pass |
| io_uring isolated, opt-in | pass (U1 + live-ring soak gated) |
| C ABI isolated (`pedradb-capi`) | pass (store `forbid`) |
| Rocks C++ not a core dep | pass |
| `mmap` / `transmute` / `unsafe impl Send` | none found |
| Miri in CI | pass on posix + `cqe_kernel` + `handles` (`scripts/miri-unsafe-islands.sh`) |

---

## Related

- RFC-0036 (`fdatasync` before Ok; Apple `F_FULLFSYNC` vs libc)
- RFC-0023 P2.2 (thin C ABI)
- RFC-0029 P1.2 (`Env::advise` / `posix_fadvise`)
- RFC-0048 W5.3/W5.4/W5.5 (F202 Seek cursor, F203 tagged CQE, U1 unique tags)
- RFC-0020 P2.5 / `docs/synthetic-field-residuals.md` (Miri residual)
- `docs/dst-seams.md` (Env is the I/O seam; io_uring is an Env impl)
- Doctrine: beat Rocks **default** (`sync=false`); Pedra still `fdatasync`s before Ok. This audit does not change that contract.
