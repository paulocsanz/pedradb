# DST seams (keep determinismo separate)

**Status:** living  
**Updated:** 2026-08-12 (World P1/P2: Net + PeerMsg + Env por peer; see `determinismo/pedradb-dst/DST-VS-FDB-SIM.md`)  


Full deterministic simulation campaigns stay in **`../determinismo`** (pedradb-dst harness, tesoura, det_io).  
This repo only ships **plug points** so a deterministic / fault-injecting host can be swapped in without rewriting the engine or store.

---

## Seams

| Concern | Trait | Production | Test / DST inject |
|---------|-------|------------|-------------------|
| **Disk I/O** | `pedradb_core::Env` (+ `EnvFile`) | `StdEnv` or `pedradb_io_uring::IoUringEnv` (Linux) | `pedradb_sim::FailingEnv`, `RecordingEnv` |
| **Time** | `pedradb_core::Clock` | `SystemClock` | `ManualClock` (shared advance) |
| **Entropy** | `pedradb_core::Rng` | `SystemRng` | `SeedRng` (replayable seed) |
| **Bundle** | `pedradb_core::Host` | `StdHost` | `DetHost<E>` (`FailingEnv` + `ManualClock` + `SeedRng`) |

**Rule:** engine/store call traits; harnesses live outside (or in `pedradb-sim` / `pedradb-dst` only).

| Layer | Env path (RFC-0015) | Notes |
|-------|---------------------|--------|
| **Kernel** | `Db::open_with_env` / flush / MANIFEST / checkpoint | Fence after failed required WAL sync; `sync_dir` errors propagate when `sync=true` |
| **DirLock** | `DirLock::release` via Env on `Db::close` / `Drop` | Drop alone remains `std::fs` best-effort if release not used |
| **Raft meta** | `persist::{store,load}_*_on(env, …)` | Path-only APIs wrap `StdEnv` |
| **WAL ship** | `append_wal_bytes_on` / `WalShipper::pull_on` / `catch_up_on` | Same |

---

## How to plug

### Kernel (PedraDB)

```rust
use pedradb_core::{Db, DetHost, OpenOptions};
use pedradb_sim::FailingEnv;

// Prefer Host when combining disk + time + entropy:
let host = DetHost::with_seed(FailingEnv::fail_after(12), 0xSEED);
let db = Db::open_with_host(path, OpenOptions { sync: true, ..Default::default() }, &host)?;

// Or Env alone:
let env = FailingEnv::fail_after(12);
let db = Db::open_with_env(path, OpenOptions { sync: true, ..Default::default() }, env)?;
```

### Store (Montanha multi-Raft)

```rust
use pedradb_core::{DetHost, SeedRng, StdEnv};
use pedradb_sim::FailingEnv;
use pedradb_store::StoreCluster;

// Deterministic election jitter (replayable).
let mut c = StoreCluster::open_with_rng(parent, 3, 2, SeedRng::new(0xDEAD))?;

// Full Host plug (env cloned per node; FailingEnv shares trip state via Rc):
let host = DetHost::with_seed(FailingEnv::passing(), 0xDEAD);
let mut c = StoreCluster::open_with_host(parent, 3, 2, &host)?;

// Explicit env + rng (same env cloned per node — shared FailingEnv trip state):
let mut c = StoreCluster::open_with_env_rng(parent, 3, 2, StdEnv, SeedRng::new(1))?;

// Per-peer Env (World P1.4): independent FailingEnv trip state per node.
use pedradb_sim::FailingEnv;
let envs = vec![FailingEnv::passing(), FailingEnv::passing(), FailingEnv::passing()];
let mut c = StoreCluster::open_with_envs_rng(parent, 3, 1, envs, SeedRng::new(1))?;
// Arm only node 1 mid-run: keep a clone of envs[0] and call arm/disarm.

// Peer RPC for World / Net (FDB-parity P1–P2): default Direct keeps unit-test sync path.
use pedradb_store::{RpcMode, PeerMsg};
c.set_rpc_mode(RpcMode::Queued);
c.advance_time(40)?; // logical clock (no wall sleep)
for (from, to, bytes) in c.drain_outbound() {
    // net.send(from, to, bytes) — same PeerMsg codec for any transport (P2.1)
    let _ = PeerMsg::decode(&bytes);
    // c.handle_inbound(from, to, &bytes)?;  // AE / RV / InstallSnapshot
}
// Membership (P2.2): c.remove_member(3)?; … c.add_member(3)?; // catch-up via InstallSnapshot
// DCS lease TTL multi-node: absolute deadline in log (not process-local table)
// c.dcs_create_ttl(b"m/lock", b"holder", 5_000)?; // expires at now_ms+5000
// c.advance_now_ms(5_000);
// assert!(c.dcs_get_on(1, b"m/lock")?.is_none());
// After pumping AE for a put that returned NotCommitted:
// c.finish_queued_propose(range_id, index, abort_if_uncommitted)?;
```

Harness: `determinismo/pedradb-dst/world` runs `RpcMode::Queued` end-to-end through `InProcessNet`.

### DCS leases

```rust
use pedradb_core::{DetHost, StdEnv};
use pedradb_dcs::{Dcs, ManualClock};
use std::time::Duration;

let clock = ManualClock::new();
let mut dcs = Dcs::open_with_clock(path, clock.clone())?;
clock.advance(Duration::from_secs(30));

// Host = disk + clock (FailingEnv + ManualClock):
let host = DetHost::with_seed(StdEnv, 7);
let mut dcs = Dcs::open_with_host(path, &host)?;
host.clock().advance(Duration::from_secs(30));
```

### Bundle (in-tree `pedradb-dst` + out-of-tree campaigns)

```rust
use pedradb_core::{Db, DetHost, Host, OpenOptions};
use pedradb_sim::FailingEnv;

let host = DetHost::with_seed(FailingEnv::from_seed(seed), seed);
host.clock().advance(std::time::Duration::from_millis(1));
let _ = host.rng().next_u64();
let db = Db::open_with_host(path, OpenOptions::default(), &host)?;
```

CI smoke: `pedradb-dst::run_seed_trial` / `sweep_seeds` (uses `open_with_host`).

---

## What is intentionally **not** in this repo

| Item | Where it lives |
|------|----------------|
| Seed→schedule fleet, shrink, repro packages | `determinismo/dst-envelope` (tesoura) |
| LD_PRELOAD lying fsync / clock | `determinismo/determinism-hooks` |
| Long property campaigns / LEDGER findings | `determinismo/pedradb-dst` |
| QEMU TCG multi-VM | `determinismo/tikv-emulation`, caixote-dst |

In-tree `pedradb-dst` crate: thin seed trials on `FailingEnv::from_seed` for CI smoke — not the full campaign runner.

---

## Gaps still open (OK for later)

| Seam | Status |
|------|--------|
| Network / RPC for multi-Raft store | Not a trait yet (in-process ticks) |
| Cooperative scheduler (thread order) | Out of scope; use single-threaded store API |
| Wire `Db`/`Store`/`Dcs` to `Host` | **Done** for open: `open_with_host` / `open_with_env_rng`; layers still free-standing after open |

---

## Related

- [RFC-0011](rfc/0011-env-fault-injection.md) — Env + FailingEnv  
- [RFC-0013](rfc/0013-montanhadb-product.md) — Montanha product  
- `crates/pedradb-core/{env,time,rng,host}.rs`  
- `crates/pedradb-sim` — media models + re-exports  
