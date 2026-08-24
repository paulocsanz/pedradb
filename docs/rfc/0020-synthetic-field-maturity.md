# RFC-0020: Synthetic field maturity (DST volume + product canaries + parallel proof lanes)

**Status:** implemented (P0–P2)  
**Updated:** 2026-08-23  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Builds on:** [RFC-0011](0011-env-fault-injection.md), [RFC-0015](0015-audit-pedradb-correctness-fixes.md), [RFC-0016](0016-pedradb-production-robustness.md), [RFC-0018](0018-fdb-method-parity-and-fault-coverage.md), [RFC-0019](0019-local-primitive-for-platform-and-scylla-need.md) (L1 done)  
**Feeds:** [RFC-0017](0017-montanha-fdb-class-substrate.md) (cluster must *survive* this program), product layers (SQL / watch / stream)  
**Out-of-tree harness (normative for tesoura / det_io / QEMU):** [`../../../determinismo/pedradb-dst/`](../../../determinismo/pedradb-dst/) — `CONFIDENCE-ROADMAP.md`, `FDB-PARITY-ROADMAP.md`, `DST-VS-FDB-SIM.md`, `findings/LEDGER.md`, `scripts/`  
**World runtime in-tree:** [RFC-0050](0050-world-in-tree-fdb-determinism.md) (P0 done) — amends boundary rule 1.  
**Seams doctrine:** [`../dst-seams.md`](../dst-seams.md)  
**Honest baseline:** [`../robustness-vs-rocks-pebble-fdb.md`](../robustness-vs-rocks-pebble-fdb.md)

---

## Background

### Facts (2026-08)

| Layer | State |
|-------|--------|
| **Pedra L1** | RFC-0019 done: CAS, seq pin, change feed, multi_get, KeyOnly, soak, backup-under-load, `compact_for_reads` |
| **Feature shape** | RFC-0014 done: leveled LSM, scan, ConcurrentDb, lz4, checkpoint, vlog+GC |
| **Fault seams** | `Env` / `Clock` / `Rng` / `Host`; `FailingEnv` OpClass + short-write + delay; `DetHost` |
| **DST method (RFC-0018)** | World + Net + buggify + CoverageMask + UCB1; overnight ops soak in-tree; cluster World in-tree (`crates/pedradb-world`) since [RFC-0050](0050-world-in-tree-fdb-determinism.md) P0 |
| **Field / multi-TB / fleet ops** | Near zero — **not** Rocks/Pebble/WiredTiger/Redwood peer |

### Pain / why now

1. **L1 is no longer the bottleneck** for Scylla-need hooks; confidence is.  
2. Unit tests + sparse soaks **do not** buy field maturity. FDB-class trust comes from **CPU-hours of seed→world→invariant→repro**, not feature checkboxes.  
3. Without a **product canary** (DB-on-top with hostile workloads), kernel passes easy cases while cluster+layer interactions stay untested.  
4. Tooling already exists **split across repos** (`pedradb` seams, `determinismo` campaigns). Missing is a **single RFC** that binds volume, parallel lanes, product invariants, and Rust test matrix into shippable slices.

### What this RFC is *not*

| Inflated claim | Honest scope |
|----------------|--------------|
| “Become RocksDB in 60 days” | **No** — multi-TB policy + decade of field lore |
| “We are FDB” | **No** — FDB-**method** (seed, buggify, silent_wrong=0, repro) |
| More CRCs as maturity | Necessary; **insufficient** without World volume + product pressure |
| Merge tesoura / det_io / QEMU into core | **Forbidden** — those campaigns stay in determinismo. **World runtime** moves in-tree under [RFC-0050](0050-world-in-tree-fdb-determinism.md) |

### Synthetic field maturity (definition)

```text
Synthetic field maturity  =
    volume of seedable worlds
  × fault-surface coverage (disk ∧ net ∧ time ∧ rng ∧ membership)
  × realistic product workloads
  × executable invariants (silent_wrong = 0)
  × every REAL → LEDGER + fix + regression seed
```

**Real field maturity** (multi-tenant cloud years) remains out of scope as a deliverable of this RFC. This program **maximizes correctness confidence per calendar day** until real deploy exists.

---

## Problems this solves

- **Problem:** No single program that turns “good lab engine” into **continuous adversary pressure** at FDB-method scale.  
- **Problem:** DST volume, codec fuzz, concurrency sanitizers, cluster partitions, and product tests are **uncoordinated** → low REAL rate.  
- **Problem:** Kernel-only tests miss **layer interactions** (CAS + feed + apply + failover + index TX).  
- **Problem:** Success is measured by features shipped, not by **trials/night, unique schedules, open REALs closed**.  
- **Problem:** New contributors don’t know which tool (World vs cargo-fuzz vs loom vs product canary) owns which bug class.

---

## Proposed solution

### Thesis

Run **eight parallel proof lanes**. Seams stay in `pedradb`; **volume and campaigns** live in `determinismo/pedradb-dst`. Ship **hostile mini-products** that only use public APIs and force limit tests under World fault schedules.

```text
┌──────────────────────────────────────────────────────────────────┐
│ LANE D — Product canaries (pedra-lease / index-tx / journal)      │
│   limit workloads → invariants → World seeds                      │
└────────────────────────────▲─────────────────────────────────────┘
                             │ public APIs only
┌──────────────────────────────────────────────────────────────────┐
│ LANE C — Montanha World (cluster sim)                             │
│   partition / leader-kill / majority / InstallSnapshot            │
└────────────────────────────▲─────────────────────────────────────┘
                             │
┌──────────────────────────────────────────────────────────────────┐
│ LANE B — Kernel hell (single-node)                                │
│   metamorphic + model oracle + OpClass fail matrix                │
└────────────────────────────▲─────────────────────────────────────┘
                             │
┌──────────────────────────────────────────────────────────────────┐
│ LANE A — Volume infra (determinismo)                              │
│   overnight soak · UCB1 · repro package · CI silent_wrong gate    │
│   (+ fuzz codecs · loom/TSan · det_io Linux as sibling lanes)     │
└──────────────────────────────────────────────────────────────────┘
```

### Eight lanes (all run in parallel once P0 gates exist)

| Lane | Name | Primary home | Owns |
|------|------|--------------|------|
| **A** | Volume World | `determinismo/pedradb-dst` | soak scale, mask novelty, nightly reports |
| **B** | Kernel hell | `pedradb-core` + `pedradb-sim` + dst harness | CAS/feed/vlog/group-commit under disk faults |
| **C** | Cluster World | `pedradb-store` + World | partitions, dual-leader fail-closed, failover |
| **D** | Product canaries | new thin crates or `examples/` + dst workloads | lease / index-tx / journal limit tests |
| **E** | Codec fuzz | `pedradb` fuzz targets or dst harness | WAL / SST / MANIFEST / CHANGELOG / PeerMsg |
| **F** | Concurrency | ConcurrentDb + store Queued | loom / TSan / careful |
| **G** | Tesoura↔World | `dst-envelope` + World | bandit schedules drive World (not only random) |
| **H** | Linux hard | det_io / QEMU scripts | CONTRACT-OK residual; io_uring path |

### Tool → bug class (use the right weapon)

| Tool | Bug class | Lane |
|------|-----------|------|
| World + FailingEnv + SeedRng | crash recovery, majority, schedules | A, B, C, D |
| Model oracle / metamorphic | silent wrong logical | B, D |
| cargo-fuzz / libFuzzer | parse/decode hang/crash | E |
| proptest / arbitrary op streams | API contract | B, D |
| loom / TSan | data races | F |
| Miri | UB (unsafe islands) | `scripts/miri-unsafe-islands.sh` (posix FFI + cqe_kernel + handles) |
| criterion (sync-labeled) | p99 regression honesty | optional P2 |
| cargo deny | supply chain | CI housekeeping |

**Order doctrine:** single-threaded logical-time World **first**; loom/TSan **complement**, never replace seed→world.

### Product canaries (force multipliers)

Not full SQL. Hostile **mini DBs** on public Pedra (+ later Montanha) APIs:

| Canary | Shape | Why it hurts the stack |
|--------|-------|------------------------|
| **pedra-lease** | `/lease/{id}` CAS + logical TTL (Clock) | hot-key CAS, crash mid-renew, double-hold forbidden |
| **pedra-index** | row + ≥2 secondary keys in one TX | half-index after crash; OCC races |
| **pedra-journal** | append subjects + change-feed consumer pin | watermark holes, ghost seq after kill |

Each canary ships with **named limit workloads** (see Acceptance). Failures produce **repro packages** (seed, schedule JSON, binary/version, exact command).

### Metrics (walls, not vibes)

| Metric | P0 bar | P1 bar | P2 stretch |
|--------|--------|--------|------------|
| CI silent_wrong gate | green on PR path for matrix entry | same + product canaries | full overnight required for merge to main (optional policy) |
| World trials / night | ≥1k (documented entry) | ≥10k | ≥100k if hardware allows |
| Coverage mask site×kind | no regression vs baseline file | novelty report in soak | UCB1 saturation tracked |
| Codec fuzz | ≥1 target continuous | all primary codecs | corpus growth tracked |
| LEDGER process | every REAL same-session | weekly triage | field provenance when deploy exists |

### Boundary rules (non-negotiable)

1. **Seams + World runtime in pedradb; tesoura / det_io / QEMU in determinismo** (`dst-seams.md`, amended [RFC-0050](0050-world-in-tree-fdb-determinism.md)). Volume *ops* soak of RFC-0020 stays in-tree `pedradb-dst`; cluster World CI is RFC-0050.  
2. **Product canaries use public APIs only** — no `pub(crate)` cheating.  
3. **silent_wrong = 0** on acked prefix; fail-stop preferred over silent continue.  
4. **Every REAL** → sibling `determinismo/pedradb-dst/findings/LEDGER.md` **and**, after RFC-0050 P1.4, the in-tree engine LEDGER + regression seed in the same effort window.  
5. **Do not claim** Rocks/Pebble/WT field parity from this RFC’s green bars.

---

## Delivery slices

### P0 — machine that kills (shippable alone)

Smallest program that **produces nightly adversary pressure** and **one product canary** under World.

- [x] **P0.1** Document + wire **CI silent_wrong gate** entry from pedradb CI (`scripts/ci_silent_wrong_gate.sh` + workflow + `silent_wrong_gate` bin); green on a fixed seed matrix — status: `done`  
- [x] **P0.2** **Overnight soak entry** runnable with `PEDRA_SOAK_TRIALS` (≥1k default); emit `volume_report.json` — status: `done` (`scripts/overnight_soak.sh`, `volume_soak` bin)  
- [x] **P0.3** **pedra-lease canary** (`crates/pedradb-lease`) on public APIs: put_if / CAS, multi_get, seq; unit tests for double-hold forbidden — status: `done`  
- [x] **P0.4** **Lease limit workloads** `hotkey-cas`, `lease-crash-reopen` (+ FailingEnv reopen); silent_wrong=0 — status: `done`  
- [x] **P0.5** **Codec fuzz P0**: WAL `WriteRecord` + CHANGELOG decode smoke (`tests/codec_fuzz_smoke.rs`); bounded CI — status: `done`  
- [x] **P0.6** Update [`../open-items.md`](../open-items.md) + README pointer; link this RFC from pedradb-dst `NEXT.md` / `CONFIDENCE-ROADMAP` “product canary” row — status: `done`  

### P1 — cluster + index pressure + concurrency + exploration

- [x] **P1.1** **pedra-index** canary: multi-key TX row+indexes; crash mid-commit never half-visible — status: `done` (`crates/pedradb-index`, W3)  
- [x] **P1.2** **Cluster partition matrix**: minority cannot commit; leader-kill catch-up silent_wrong=0 — status: `done` (`pedradb-store` rfc20_*)  
- [x] **P1.3** **Explore schedule** drives campaigns (rotating offsets + optional sibling tesoura) — status: `done` (`scripts/explore_campaign.sh`)  
- [x] **P1.4** **ConcurrentDb race job** multi-thread stress + optional TSan path — status: `done` (`scripts/race_job.sh`, `concurrent_race_stress`)  
- [x] **P1.5** Fuzz smokes for SST + MANIFEST + PeerMsg — status: `done`  
- [x] **P1.6** Soak ≥10k ops mode + volume report — status: `done` (`PEDRA_SOAK_MODE=ops`, `PEDRA_SOAK_MIN=10000`)  

### P2 — journal canary, volume stretch, Linux hard, honesty benches

- [x] **P2.1** **pedra-journal** canary + `feed-watermark` — status: `done` (`crates/pedradb-journal`)  
- [x] **P2.2** Stretch soak config (≥100k ops) + weekly LEDGER triage checklist — status: `done` (`scripts/stretch_soak.sh`, `docs/synthetic-field-residuals.md`)  
- [x] **P2.3** **Linux det_io / QEMU** residual recorded (entries exist; hard bar blocked without Linux/guest) — status: `done`  
- [x] **P2.4** Honesty bench entry documented (`cargo bench -p pedradb-core --bench baseline`) — status: `done`  
- [x] **P2.5** Miri on unsafe islands — status: `done` (`scripts/miri-unsafe-islands.sh`: posix FFI + `cqe_kernel` + capi `handles`; CI `MIRI_REQUIRED=1`)  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CI silent_wrong gate wired/documented | done | scripts/ci_silent_wrong_gate.sh + workflow | 2026-08-13 |
| P0.2 | p0 | Overnight soak ≥1k + volume_report | done | scripts/overnight_soak.sh + volume_soak bin | 2026-08-13 |
| P0.3 | p0 | pedra-lease canary crate/example | done | crates/pedradb-lease | 2026-08-13 |
| P0.4 | p0 | Lease limit workloads under World/sim | done | hotkey-cas + lease-crash-reopen | 2026-08-13 |
| P0.5 | p0 | Codec fuzz P0 (WAL or CHANGELOG) | done | codec_fuzz_smoke tests | 2026-08-13 |
| P0.6 | p0 | Index/docs cross-link | done | this doc + README/open-items/dst | 2026-08-13 |
| P1.1 | p1 | pedra-index canary + crash mid-TX | done | crates/pedradb-index | 2026-08-13 |
| P1.2 | p1 | Cluster partition + leader-kill matrix | done | store rfc20_* tests | 2026-08-13 |
| P1.3 | p1 | Explore/bandit → campaign | done | scripts/explore_campaign.sh | 2026-08-13 |
| P1.4 | p1 | ConcurrentDb race job | done | race_job.sh + concurrent_race_stress | 2026-08-13 |
| P1.5 | p1 | Fuzz SST/MANIFEST/PeerMsg | done | codec_fuzz_smoke_p1 + peer_msg | 2026-08-13 |
| P1.6 | p1 | 10k trials/night path + report | done | ops soak + overnight_soak.sh | 2026-08-13 |
| P2.1 | p2 | pedra-journal canary | done | crates/pedradb-journal | 2026-08-13 |
| P2.2 | p2 | 100k soak + weekly LEDGER triage | done | stretch_soak.sh + residuals doc | 2026-08-13 |
| P2.3 | p2 | Linux det_io hard bar | done | residual documented | 2026-08-13 |
| P2.4 | p2 | Sync-labeled benches | done | criterion baseline entry | 2026-08-13 |
| P2.5 | p2 | Miri on unsafe crates | done | `scripts/miri-unsafe-islands.sh` + supply-chain job | 2026-08-23 |

---

## Acceptance criteria

### Tests (named — must exist for “done” slices)

**P0**

- [x] CI (or documented equivalent) fails if silent_wrong ≠ 0 on the gate matrix.  
- [x] `overnight` / soak entry produces `volume_report.json` with trials ≥ 1000 and silent_wrong = 0.  
- [x] `pedra-lease`: concurrent CAS → exactly one winner; crash after Ok → only durable holder.  
- [x] Workloads `hotkey-cas`, `lease-crash-reopen` green under FailingEnv or World seed matrix.  
- [x] Fuzz target builds and runs bounded seconds in CI without crash on seed corpus.

**P1**

- [x] Index TX crash: recover shows all-or-nothing secondary keys.  
- [x] Partition: minority write does not become majority-visible; leader-kill catch-up silent_wrong=0.  
- [x] ≥1 explore/bandit schedule integrates with campaign (in-tree + optional sibling).  
- [x] ConcurrentDb race job green (TSan optional residual path).  
- [x] Additional codec fuzz targets present for SST/MANIFEST/PeerMsg as claimed.

**P2**

- [x] Journal consumer pin: no feed entry with seq > durable last after kill.  
- [x] Stretch soak config documented; weekly triage in `docs/synthetic-field-residuals.md`.  
- [x] det_io/QEMU status recorded (pass or residual with ticket).  

### Telemetry / analytics

- **Required artifacts:** `volume_report.json`, CoverageMask snapshot (or hash), soak stdout summary.  
- **Optional:** wal_sync_count / compact_count from kernel stats inside product trials.  
- **None for product UX** — backend confidence program.

### Documentation

- This RFC status table updated **in the same change** as code.  
- [`../dst-seams.md`](../dst-seams.md) remains the seam boundary.  
- `determinismo/pedradb-dst/NEXT.md` points here for product canaries + volume program.  
- [`../open-items.md`](../open-items.md) “next action” includes RFC-0020.  
- README maturity row: synthetic field program (not “Rocks parity”).

### Screenshots

- backend-only (reports/JSON acceptable as evidence).

---

## Named limit workloads (catalog — implement per canary)

| ID | Workload | Limit | Invariant |
|----|----------|-------|-----------|
| W1 | `hotkey-cas` | N writers, 1 key | linear CAS history; no lost update |
| W2 | `lease-crash-reopen` | kill after Ok | only durable holder |
| W3 | `index-tx-crash` | crash mid multi-key | 0 or full index set |
| W4 | `feed-watermark` | kill after commits | feed ⊆ last_seq; no ghosts |
| W5 | `l0-storm-read` | many flushes + compact_for_reads | get/scan = model |
| W6 | `enospc-mid-compact` | StorageFull on rename | fence or recover; silent_wrong=0 |
| W7 | `partition-majority` | minority partition | no false commit |
| W8 | `leader-kill-apply` | kill after majority | catch-up correct |
| W9 | `huge-scan-limit` | large keyspace + limit | no OOM; order stable |
| W10 | `vlog-churn` | large values + compact_vlog mid-crash | resolve or fail-closed |
| W11 | `backup-under-storm` | put + ship_wal | restore ⊆ acked |

P0 requires W1–W2. P1 adds W3, W7–W8. P2 adds W4 (journal) and stretches W5–W11 as capacity allows.

---

## Relationship to other RFCs

| RFC | Role vs 0020 |
|-----|----------------|
| **0018** | Method + seam inventory + buggify infrastructure — **prerequisite**; 0020 **adds volume + products + parallel lanes** |
| **0019** | L1 API completeness — **done**; canaries **consume** CAS/seq/feed |
| **0017** | Montanha substrate features — 0020 **stresses** them; does not replace multi-Raft product work |
| **0016** | Engine robustness features — 0020 **proves** them under volume |
| **0010** | DBs on top charter — 0020 **instantiates** hostile micro-products |

---

## Out of scope

- Claiming production parity with RocksDB, Pebble, WiredTiger, or FDB Redwood.  
- Full Postgres / CQL / ClickHouse wire compatibility.  
- Multi-region fleet backup, encryption-at-rest product, or real customer deploy.  
- Absorbing `determinismo` into the pedradb crate graph.  
- Dual durable B-tree + LSM primary.  
- Replacing human field ops with simulation alone.  
- Fixing GitHub Actions billing (ops; not a proof lane).

---

## How to run (P0–P2 shipped)

```bash
# P0
bash scripts/ci_silent_wrong_gate.sh
PEDRA_SOAK_MODE=ops PEDRA_SOAK_TRIALS=1000 bash scripts/overnight_soak.sh volume_report.json
cargo test -p pedradb-lease -- --nocapture
cargo test -p pedradb-core --test codec_fuzz_smoke -- --nocapture

# P1
cargo test -p pedradb-index -- --nocapture
cargo test -p pedradb-store rfc20_ -- --nocapture
bash scripts/explore_campaign.sh
bash scripts/race_job.sh
cargo test -p pedradb-core --test codec_fuzz_smoke_p1 -- --nocapture
cargo test -p pedradb-store --test peer_msg_fuzz_smoke -- --nocapture
PEDRA_SOAK_MODE=ops PEDRA_SOAK_TRIALS=10000 PEDRA_SOAK_MIN=10000 bash scripts/overnight_soak.sh volume_report_10k.json

# P2
cargo test -p pedradb-journal -- --nocapture
PEDRA_SOAK_MODE=ops PEDRA_SOAK_TRIALS=100000 bash scripts/stretch_soak.sh volume_report_stretch.json
# residuals: docs/synthetic-field-residuals.md
```

CI: `.github/workflows/synthetic-field.yml`

## Suggested parallel execution (first two weeks)

| Owner focus | Slices |
|-------------|--------|
| DST / determinismo | P0.1, P0.2 — **done** |
| Kernel + product API | P0.3, P0.4 — **done** |
| Fuzz | P0.5 — **done** |
| Docs | P0.6 — **done** |
| (After P0 green) | P1.1 + P1.2 + P1.4 in parallel |

P0 is **useful alone**: a green silent_wrong gate + 1k soak + lease canary already changes the confidence trajectory. P1/P2 expand surface and volume.

---

## One-line north star

> **Maximize REAL bugs found per CPU-hour under seedable worlds and product-shaped load — until the only maturity left is real field time.**
