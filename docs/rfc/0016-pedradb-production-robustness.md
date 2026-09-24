# RFC-0016: PedraDB production robustness (launch-ready engine)

**Status:** done (P0–P2; P1.4 rocks-parity-bench; P2.1 ops backup-under-load; P2.2 ingest)  
**Updated:** 2026-08-24  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Builds on:** [RFC-0014](0014-rocks-pebble-redwood-maturity.md) (feature shape — **done**), [RFC-0015](0015-audit-pedradb-correctness-fixes.md) (audit correctness — **done**), [RFC-0011](0011-env-fault-injection.md), [RFC-0012 research](0012-research-decisions.md)  
**Complements:** [RFC-0017](0017-montanha-fdb-class-substrate.md) (distributed product on top of this kernel), [RFC-0019](0019-local-primitive-for-platform-and-scylla-need.md) (CAS / seq pin / change feed for platform L1)  
**Doctrine:** [`../performance-ceiling-option-preservation-and-sled-layer.md`](../performance-ceiling-option-preservation-and-sled-layer.md), [`../robustness-vs-rocks-pebble-fdb.md`](../robustness-vs-rocks-pebble-fdb.md)

---

## Background

RFC-0014/0015 closed **feature shape** and **structural durability seams** (fence, `sync_dir`, Env, leveled LSM, lazy blocks, OCC, vlog *spill*, local backup). That is **not** enough to launch as a peer of RocksDB, Pebble, WiredTiger, or Redwood-as-storage.

Honest position today:

| Axis | Lab / shipped | Production peer gap |
|------|---------------|---------------------|
| Correctness contracts | Strong on happy + injected disk path | Field lore, lying disks, multi-year bug tax |
| Write amp / p99 | Leveled *shape*; serial put+fsync | Group commit, compact pacing, multi-TB proof |
| Large values | Spill to `VALUES.vlog` | **No GC** → disk grows forever |
| Concurrency | OCC + coarse write lock | Multi-memtable / background compact / real multi-core QPS |
| Ops | Checkpoint, local PITR, verify | Continuous backup under load, repair, ingest, encryption |
| Proof | LEDGER + denser schedules | Simulation + soak + apples-to-apples benches |

**Why now:** feature checkboxes without production robustness produce a sled-class launch (clever, incomplete, untrusted). This RFC is the **engine** track so that when Montanha/apps launch, PedraDB is not the weak link.

### Deferred value-log GC (context for this RFC)

With `OpenOptions::large_value_threshold = Some(n)`:

1. Puts with `value.len() ≥ n` append to `VALUES.vlog` and store a compact `VLG1` pointer in WAL/mem/SST.  
2. Compaction of SSTs **does not rewrite** large payloads (write-amp win for large values).  
3. **GC is deferred:** when a key is overwritten or deleted, the old vlog record remains on disk. The file **only grows**.

**Consequences of deferred GC (today):**

| Consequence | Severity | Mitigation until P0.1 ships |
|-------------|----------|------------------------------|
| Disk use ≈ sum of *all historical* large values | High under churn | Disable threshold, or re-base backup + wipe data dir, or rotate DB dir |
| Checkpoint/backup size includes full vlog | High | Document; full copy of `VALUES.vlog` required for correctness |
| Space amp can exceed classic LSM amp | High for update-heavy large-value workloads | Prefer inline for small/medium values; measure before enabling |
| No reclaim after delete of large keys | Medium–high | Same as above |
| Crash consistency of vlog+pointer | Handled if vlog append+fsync before WAL Ok | Keep contract; DST must cover vlog mid-append |

**Not a correctness bug** for live keys (get/scan resolve live pointers). It **is** a production **ops bomb** if large-value mode is default under update-heavy load. Closing GC is P0 of this RFC when large values are a launch workload; otherwise document “threshold = opt-in lab only.”

---

## Problems this solves

- **Problem:** Launch narrative claims Rocks/Pebble-class without write-path and compaction that survive real load.  
- **Problem:** Large-value path ships without reclaim → silent disk fill.  
- **Problem:** OCC exists but fsync serialization still caps multi-core writes.  
- **Problem:** DST density and ENOSPC/EIO under concurrent compact are thinner than competitors’ lore.  
- **Problem:** No launch gate (bench + soak + fault suite) that blocks “ship” when red.

---

## Proposed solution

A **production robustness wave** on the embed kernel only (Montanha is RFC-0017):

1. **Value-log GC** — reclaim unreferenced vlog extents (or rewrite + swap) with correctness under crash.  
2. **Write-path maturity** — group commit / batch fsync amortization; measured QPS under concurrent OCC.  
3. **Compaction of war** — pacing, amp metrics, L0 policy, background compact without blocking puts forever.  
4. **Fault & soak program** — denser DST + multi-GB/TB lab soak; silent_wrong=0 gate.  
5. **Ops hardening** — continuous backup under write load; verify; honest usage for large values.  
6. **Launch scoreboard** — published comparison benches (sync apples-to-apples) + “go / no-go” checklist.

Preserve anti-corner rules (versioned formats, no dual primary B-tree, no knob zoo).

---

## Delivery slices

### P0 — stop the bombs (shippable alone)

- [x] **P0.1** Value-log GC (or rewrite-compact) for unreferenced records; crash-safe; tests under delete/overwrite + reopen — status: `done` (`Db::compact_vlog`, MANIFEST `vlog_use_new`, mid-GC reopen tests)  
- [x] **P0.2** Document + enforce: threshold off by default; usage warns when vlog size ≫ live data; stats expose `vlog_bytes` / live estimate — status: `done`  
- [x] **P0.3** Amp + durability metrics: bytes written / ingested, compact duration, fsync count; surface on `DbStats` or ops — status: `done` (`DbStats` amp + vlog fields)  
- [x] **P0.4** Launch soak script: N-hour put/get/delete/flush/compact under FailingEnv schedule; silent_wrong vs model — status: `done` (core StdEnv soak + `pedradb-sim` FailingEnv soak)  

### P1 — write path & compaction peer pressure

- [x] **P1.1** Group commit (or concurrent-writer fsync amortization) with tests: multi-thread puts; durability contract unchanged — status: `done` (`ConcurrentDb` write group + `Db::group_commit`)  
- [x] **P1.2** Dual-memtable switch + ConcurrentDb flush SST I/O off write lock (puts not starved) — status: `done`  
- [x] **P1.3** Pipeline: imm flush while active mem accepts writes; compact after flush pipeline — status: `done`  
- [x] **P1.4** Apples-to-apples bench harness vs Rocks/fjall + multi-GB lab — status: `done` (`crates/rocksdb-parity-bench` + `scripts/rocksdb_parity_v0.sh`; official peer Rocks `sync=false`; fjall is not the official peer)  

### P2 — ops & launch readiness polish

- [x] **P2.1** Continuous backup under load (ship_wal / checkpoint without wrong live reads) — status: `done` (`rfc19_backup_under_continuous_put_restore_acked_prefix` in `pedradb-ops`)  
- [x] **P2.2** Optional SST ingest / bulk load path (if Montanha snapshot install needs it) — status: `done` (RFC-0050 P0.6: `SstFileWriter` / `ingest_external_file`)  
- [x] **P2.3** Encryption-at-rest hook **or** explicit non-goal with layer recommendation — status: `done` (explicit non-goal: app/FS layer)  
- [x] **P2.4** Public “launch readiness” checklist in `docs/usage.md` + robustness doc update — status: `done`  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Value-log GC crash-safe | done | `compact_vlog` + MANIFEST `vlog_use_new` | 2026-08-12 |
| P0.2 | p0 | Vlog ops docs + stats | done | usage.md + `vlog_*` stats | 2026-08-12 |
| P0.3 | p0 | Amp / fsync metrics | done | `DbStats` amp counters | 2026-08-12 |
| P0.4 | p0 | Soak + silent_wrong gate | done | core + FailingEnv soak | 2026-08-12 |
| P1.1 | p1 | Group commit / fsync amortize | done | ConcurrentDb WriteGroup | 2026-08-12 |
| P1.2 | p1 | Fine write lock / flush off-lock | done | dual-mem + prepare_flush_imm | 2026-08-12 |
| P1.3 | p1 | Dual-mem pipeline | done | imm + active mem | 2026-08-12 |
| P1.4 | p1 | Bench harness + multi-GB lab | done | rocksdb-parity-bench vs Rocks default | 2026-08-24 |
| P2.1 | p2 | Backup under load | done | ops `rfc19_backup_under_continuous_put_restore_acked_prefix` | 2026-08-24 |
| P2.2 | p2 | SST ingest (if needed) | done | RFC-0050 `ingest_external_file` | 2026-08-24 |
| P2.3 | p2 | Encryption decision | done | non-goal: FS/app layer | 2026-08-12 |
| P2.4 | p2 | Launch checklist docs | done | usage.md checklist | 2026-08-12 |

---

## Acceptance criteria

### Tests

**P0**

- [x] Overwrite/delete large values → after GC, vlog size decreases or live ratio improves; reopen get correct.  
- [x] Crash mid-GC: reopen never returns wrong live value; fail-closed or prior version (MANIFEST `vlog_use_new`; tests before/after MANIFEST).  
- [x] Stats show vlog growth before GC and improvement after.  
- [x] Soak: fixed seed schedule, silent_wrong=0 vs model.

**P1**

- [x] Concurrent puts under group-commit path: Ok ⇒ durable; higher sustained QPS than pre-change under same machine (number attached to PR).  
- [x] Compact pacing: put latency does not wedge unbounded during L0 storm (bounded test).  
- [x] Bench doc: table of thruput/latency vs Rocks/fjall with sync modes labeled.

**P2**

- [x] Backup ship during continuous put stream; restore verifies acked prefix.  
- [x] Launch checklist exists and is referenced from README/open-items.

### Telemetry / analytics

- In-process: `vlog_bytes`, compact amp counters, fsync counts (P0.3).  
- None external required for kernel.

### Documentation

- This RFC status table.  
- `docs/usage.md`: large-value + GC behavior; launch checklist (P2.4).  
- `docs/robustness-vs-rocks-pebble-fdb.md`: residual walls updated when P0/P1 land.  
- Cross-link RFC-0017 (Montanha must not depend on unshipped P0.1 if it enables large values by default).

### Screenshots

- backend-only (optional flamegraph/bench PNG in PR, not required in tree).

---

## Out of scope

- Claiming field parity without field time (this RFC only builds *readiness*).  
- Dual on-disk B-tree + LSM (performance-ceiling X4).  
- Full Monkey/Lazy Leveling as default without metrics (0012 discipline).  
- Distributed multi-Raft / FDB Simulation (**RFC-0017**).  
- Becoming a SQL database in core.

---

## Relationship to “destroy the competition”

Competitors lose when **we ship earlier with honest contracts** and **prove** durability + predictable p99 on the workloads we choose (embed ACID + substrate for Montanha). They keep winning on decade-long lore until we:

1. Do not ship a disk-fill vlog.  
2. Publish numbers under honest sync.  
3. Keep silent_wrong hunts as a standing gate.  
4. Let Montanha (0017) own the distributed story while Pedra owns the local kernel.

**P0 of this RFC is the minimum before marketing large values or multi-writer as production defaults.**
