# RFC-0013: MontanhaDb product specification

**Status:** done (P0–P2; Live/metrics/leases/cluster-id shipped; TLS via 0050; World swarm via 0059)  
**Updated:** 2026-08-24  
  
**ID:** 0013  
**Product name:** MontanhaDb (Montan-HA-DB); short: **Montanha**  
**Kernel (separate product):** PedraDB — [RFC-0001](0001-pedradb-high-level-spec.md)  
**Doctrine companion:** [montanha-vs-foundationdb.md](../montanha-vs-foundationdb.md)  
**Layering companion:** [montanha-layering-dcs-on-store.md](../montanha-layering-dcs-on-store.md)  
**Supersedes as product north star:** informal product notes in `montanhadb.md` where they conflict — **this RFC wins after approval**

---

## How to use this RFC

| Actor | Duty |
|-------|------|
| **Implementer** | Ship only slices in **Delivery**; mark Status `done` only when **slice acceptance** + **global invariants** hold |
| **Validator / skeptic** | Attempt to falsify claims using **Verification matrix** (§8) and **Deep test catalogue** (§9); green suite alone is insufficient if a named invariant is untested |
| **Product** | Approve doctrine (§2–§4); reject PRs that re-identity Montanha as “etcd product” or “TiKV clone” |

**Pass bar for “Montanha P0 done”:** all P0 checkboxes `done`, Status table updated, every P0 row in §8 green with **saved evidence** (CI log or scratch log path cited in PR), docs listed in §10 for P0 present and non-contradictory.

---

## 1. Background

### 1.1 Facts in repo today (not aspirations)

| Component | Location | What it actually is |
|-----------|----------|---------------------|
| PedraDB | `pedradb-core` | Embed ordered KV + multi-key TX; one process per directory |
| Bootstrap Raft + DCS apply | `pedradb-raft`, `pedradb-dcs`, `pedra-raft-node` | Single-domain multi-process Raft; DcsCommand apply; network demo path |
| Store MVP | `pedradb-store` | In-process multi-node multi-**range** Raft; put/get; strong/local read; DCS create/cas on store; partition/failover tests |
| Product prose | `docs/montanhadb*.md`, layering, FDB compare | Doctrine exists; **no prior complete product RFC** |
| Kernel RFCs | 0001–0012 | PedraDB + outer rungs; **not** Montanha product contract |

### 1.2 Pain / why now

1. **No single contract** for “what Montanha must be” → implementers oscillate between etcd-shaped DCS, TiKV-shaped multi-Raft identity, and FDB-shaped layers.  
2. **Validation without a target** → tests pass while product identity drifts; skeptic panels re-litigate architecture every session.  
3. **User need:** a document detailed enough to **implement against** and **adversarially validate** (correctness, tests, docs).

### 1.3 Related docs (status after this RFC)

| Doc | Status relative to 0013 |
|-----|-------------------------|
| [montanhadb.md](../montanhadb.md) | Executive summary; must link here; must not contradict |
| [montanha-vs-foundationdb.md](../montanha-vs-foundationdb.md) | Doctrine detail; normative for FDB vs TiKV language |
| [montanha-layering-dcs-on-store.md](../montanha-layering-dcs-on-store.md) | DCS-on-store rules; subordinate to §4.3 here |
| [live-leadership-and-patroni-shaped-ha.md](../live-leadership-and-patroni-shaped-ha.md) | Live plane design; P1/P2 |
| RFC-0010 / 0012 | Historical outer rungs / bootstrap multi-node; **not** Montanha end-state identity |

---

## 2. Problems this solves

1. **Problem:** Need a **FoundationDB-shaped** multi-node product on PedraDB: transactional ordered KV substrate + **layers**, not a second etcd.  
2. **Problem:** Need **named correctness** (majority durability, fencing, read models) so HA agents and apps do not split-brain.  
3. **Problem:** Need a **horizontal write path** without forcing “one Raft group for all user data forever.”  
4. **Problem:** Coordination (leader locks, config) must be **data on the substrate**, not a forever-special consensus universe that becomes the brand.  
5. **Problem:** Engineering and review need **falsifiable acceptance** (deep tests + docs), not vibes.

---

## 3. Proposed solution (product definition)

### 3.1 One sentence (normative)

**MontanhaDb is a multi-node, FoundationDB-shaped data platform built on PedraDB: an ordered key-value substrate with strongly defined durability and consistency, plus thin layers for coordination and higher models — never “etcd the product,” never “TiKV the brand.”**

### 3.2 Stack (normative)

```text
┌──────────────────────────────────────────────────────────────┐
│  L3  Apps / HA agents / controllers                          │
│      (Patroni-shaped agents, operators, business apps)       │
├──────────────────────────────────────────────────────────────┤
│  L2  Layers (libraries or thin services)                     │
│      Montanha-DCS · Montanha-Live · (future SQL/record…)     │
├──────────────────────────────────────────────────────────────┤
│  L1  Montanha-Store  — distributed ordered KV substrate      │
│      locate · put/get · (TX path as required by slices)      │
│      replication/consensus is PLUMBING, not product identity │
├──────────────────────────────────────────────────────────────┤
│  L0  PedraDB — local engine per peer (RFC-0001)              │
│      one directory · one process · multi-key ACID locally    │
└──────────────────────────────────────────────────────────────┘
```

| Layer | Must | Must not |
|-------|------|----------|
| **L0 PedraDB** | Embed kernel only | Multi-node, network, Raft inside core |
| **L1 Store** | Distributed substrate apps/layers use | Be marketed as “the DCS” |
| **L2 DCS** | create/CAS/get/revision semantics on keys | Own a permanent separate data-plane consensus for *all* data |
| **L2 Live** | Best-effort streams of leadership/membership | Be fencing truth alone |
| **L3 Apps** | Use L1/L2 APIs | Open same PedraDB dir from two processes |

### 3.3 Product identity rules (normative language)

| Allowed | Forbidden as identity |
|---------|------------------------|
| “FDB-shaped: TX/KV substrate + layers” | “We are a better etcd” |
| “Multi-Raft (or other) under the hood for replication” | “Montanha = TiKV clone / TiKV wire” |
| “DCS is a layer on the store” | “DCS *is* Montanha” |
| “PedraDB is the local kernel” | “PedraDB is the distributed database” |

**Replication plumbing (informative, not identity):**  
Current preferred P0/P1 implementation for L1 durability across peers is **Raft groups over key ranges** (multi-Raft), because it is implementable and testable in-tree. Future replacement of that plumbing (e.g. different consensus packaging) is allowed **if** all normative invariants in §5 still hold and tests in §9 still pass. Product docs must not rename Montanha to “the multi-Raft database.”

### 3.4 FoundationDB kinship (normative scope of “like FDB”)

| Like FDB (required direction) | Not required (non-goals of this RFC) |
|-------------------------------|--------------------------------------|
| Ordered keys + transactional substrate as core | FDB wire protocol / fdbcli |
| Layers for rich models and coordination | Record Layer / Document Layer clones |
| Coordination as keys + conditional writes | FDB Simulation at Apple scale |
| Apps share one substrate | FDB unbundled proxy/resolver/tlog role split day one |
| Honest limits documented | Production pedigree claims |

---

## 4. Surfaces and APIs (normative requirements)

### 4.1 Montanha-Store (L1) — required capabilities

Implementations **must** expose (names may be Rust-idiomatic; semantics fixed):

| Capability | Semantics |
|------------|-----------|
| **Cluster open** | N≥3 peers (test default 3), each with private PedraDB directory under a parent path |
| **Keyspace ranges** | ≥2 independent ranges covering full keyspace; every key maps to exactly one range (`locate`) |
| **Range leadership** | Each range has at most one **safe** live leader for client routing; dual Leader claims ⇒ **no** safe leader (fail closed) |
| **`put(key, value)`** | Routed to range leader; returns **Ok only if** entry is **majority-committed** and applied on a strict majority path as defined in §5.1; otherwise typed error (e.g. `NotCommitted` / `NotLeader`) — **never silent Ok** |
| **`get` / read policies** | At least two named policies: `LocalApplied` (non-linearizable) and `Strong` (leader / revalidated only) — §5.2 |
| **Participation / HA** | Ability to remove a peer from participation (partition/crash model); remaining majority elects; committed data retained |
| **Discard uncommitted client proposes** | Failed majority propose must not leave orphans that later commit without a client Ok (fencing / dual-create brick) |

**TX on store (staged):**

| Stage | Requirement |
|-------|-------------|
| **P0** | Single-key put/get + DCS conditional ops sufficient for locks; multi-key **local** TX remains PedraDB |
| **P1** | Documented path for multi-key atomicity **within one range** (TX or batch through Raft) with tests |
| **P2** | Cross-range atomicity strategy **documented** (2PC / exclusive ranges / reject); implementation optional |

### 4.2 Montanha-DCS (L2) — required capabilities

DCS is a **layer on L1**, not a second mountain.

| Op | Semantics |
|----|-----------|
| **Create(key, value)** | Succeeds iff key logically absent; on success, value + **mod_revision** durable per §5; second Create fails |
| **Cas(key, value, expected_rev)** | Succeeds iff current mod_revision matches (0 = create-if-absent class if exposed) |
| **Get** | Returns value + create/mod revision; available on peers after commit |
| **Propose path** | Leader pre-check → log/commit via **same range Raft (or L1 commit path)** that owns the key → apply on all committing peers |
| **Fencing** | Callers use **mod_revision** (or explicit token derived from it) as fence; Live stream is not fence |

**Leases (staged):** P0 may use lease=0 only; P1+ defines durable-or-fail-safe lease semantics (no immortal locks across process restart without explicit design — see F7 lessons in `pedradb-dcs`).

### 4.3 Montanha-Live (L2) — required capabilities (P1+)

| Op | Semantics |
|----|-----------|
| **Subscribe** | Best-effort stream of leadership/key changes |
| **Truth** | Agents **must** still renew/CAS; demote on failed renew |
| **Honesty** | API and docs state **best-effort / non-fencing** |

### 4.4 Bootstrap path (allowed, non-identity)

Single-domain `pedradb-raft` + `DcsCommand` + `pedra-raft-node` **may** remain for network demos and regression.  

**Normative:** New coordination features and product claims for Montanha **prefer L1 store**; bootstrap must not expand into the general data plane.

### 4.5 Wire / binary (staged)

| Stage | Requirement |
|-------|-------------|
| **P0** | Library-level in-process multi-peer store is sufficient for correctness proof |
| **P1** | At least one **documented** way to run ≥3 peers (process or documented harness) for store **or** keep explicit “lib-only bar” in Status if deferred |
| **P2** | Production-oriented binary, auth, TLS — out of P0 |

---

## 5. Normative correctness invariants

These are **product laws**. A slice is not `done` if any invariant it claims is untested or violated.

### 5.1 Majority durability (writes)

**I-MAJ-1:** If `put` / DCS mutate returns **Ok**, then a strict majority of the range’s **configured** membership has the entry’s effect in **applied** PedraDB state (or equivalent applied state machine), within the test’s observation model (immediate after Ok for in-process MVP).

**I-MAJ-2:** A minority of peers alone must **not** be able to make the client API return Ok for a new write (partition tests).

**I-MAJ-3:** Client **Err** (not committed) must not later become durable on majority **without a new successful client Ok** (no silent orphan commit after discard).

**I-MAJ-4:** Uncommitted / discarded entries must not brick the apply pipeline (applied cursor reaches commit; subsequent puts succeed).

### 5.2 Read models

**I-RD-1:** Every public read API names its model: `LocalApplied` or `Strong` (or documented alias).

**I-RD-2:** `Strong` must **fail closed** if the serving node is not the unique safe range leader (including dual Leader claims).

**I-RD-3:** Docs must state `LocalApplied` is **not** linearizable and must not be used for fencing.

### 5.3 Range HA

**I-HA-1:** After loss of current range leader participation, a new leader is elected among remaining majority-capable membership (for N=3, 2 live peers).

**I-HA-2:** Keys majority-committed before failure remain readable on a majority of **remaining** peers after failover.

**I-HA-3:** New puts after failover can return Ok under majority.

### 5.4 Multi-range write power

**I-MR-1:** ≥2 ranges can each accept puts (under their leaders) without requiring a single global writer for the whole keyspace.

**I-MR-2:** `locate(key)` is total for all byte keys in the configured split.

### 5.5 DCS / fencing

**I-DCS-1:** Create-if-absent: at most one winner; loser fails; winner replicated to peers after Ok.

**I-DCS-2:** Conflicting Create after success fails at pre-check or equivalent.

**I-DCS-3:** CAS with wrong revision fails; correct revision advances mod_revision.

**I-DCS-4:** DCS mutations use L1 commit path (same durability laws as put).

**I-DCS-5:** NotCommitted create + heal + tick does **not** install the lock without Ok; retry after heal can Ok; subsequent puts not bricked.

### 5.6 PedraDB boundary

**I-PEDRA-1:** Montanha must not require multi-process open of one PedraDB directory.

**I-PEDRA-2:** Kernel remains free of network/Raft (RFC-0001).

---

## 6. Delivery slices

Status values: `todo` | `doing` | `done` only.

### P0 — substrate correctness + DCS layer proof (must ship first)

Smallest useful Montanha: **correct multi-range store + DCS-on-store + deep tests + canonical docs**.

- [x] **P0.1** Normative docs land: this RFC approved path + `montanhadb.md` points here as contract — status: `done`  
- [x] **P0.2** Multi-range locate + concurrent range puts (I-MR-*) — status: `done`  
- [x] **P0.3** Majority-durable put + NotCommitted under minority + no orphan commit (I-MAJ-*) — status: `done`  
- [x] **P0.4** Named Strong vs LocalApplied reads; dual-leader fail closed (I-RD-*) — status: `done`  
- [x] **P0.5** Range leader loss → re-elect → put + retain commits (I-HA-*) — status: `done`  
- [x] **P0.6** DCS create/cas/get on store + NotCommitted heal/retry no brick (I-DCS-*) — status: `done`  
- [x] **P0.7** Deep test suite + CI gate `cargo test -p pedradb-store --lib` (and dcs apply tests as needed) green; clippy `-D warnings` on store path — status: `done`  
- [x] **P0.8** Operator/developer doc: how to run tests, invariant map, consistency names — status: `done`

### P1 — productization of the substrate

- [x] **P1.1** Single-range multi-key atomic batch/TX through L1 commit path + tests — status: `done`  
- [x] **P1.2** Montanha-Live MVP: subscribe best-effort + docs that it is non-fencing + test double-plane (truth vs stream) — status: `done` (`subscribe_leadership` / `LeadershipEvent`; stream is **not fencing**; test `live_hub_failover_notifies_without_polling_dcs`)  
- [x] **P1.3** Cluster id / membership metadata keys (documented layout) + refuse silent cross-cluster merge — status: `done`
      (`\0store/cluster/id` 16 bytes + `\0store/cluster/membership` u32+u64s;
      `StoreError::ClusterMismatch` on mixed node dirs; pin via
      `StoreOpenOptions::with_cluster_id` / `montanha-tcp --cluster-id`;
      tests `cluster_id_survives_reopen`, `cluster_id_refuses_cross_cluster_node_dir`)  
- [x] **P1.4** Network or multi-process store harness **or** explicit deferral recorded in Status with reopen criteria — status: `done` (deferred; see Status)  
- [x] **P1.5** Metrics hooks (counters: commits, NotCommitted, elections) **or** explicit “none — library MVP” with issue link — status: `done` (`StoreMetrics` / `StoreCluster::metrics`; test `store_metrics_count_commits_and_elections`)  
- [x] **P1.6** Lease story for DCS (fail-safe on restart) aligned with pedradb-dcs F7 — status: `done` (`lease_kernel` + persisted `now_ms`; test `dcs_lease_fail_safe_on_reopen`)

### P2 — climb (not required for “P0 Montanha”)

- [x] **P2.1** Cross-range TX strategy doc + optional prototype — status: `done` (`commit_tx` 2PC shipped; not full FDB OCC)  
- [x] **P2.2** Placement / split / merge automation beyond static splits — status: `done` (`split_range_at` + `merge_adjacent_ranges`; tests `split_range_at_two_ranges_put`)  
- [x] **P2.3** Production binary packaging, TLS, auth — status: `done` (RFC-0050 P0.5 / RFC-0021: `montanha-tcp --tls-*` / `--require-tls`; not GA-default)  
- [x] **P2.4** Optional etcd-shaped façade (Kine-like) on store — status: `done` (`EtcdNeedFace` create/CAS/get + watch; RFC-0022 P0.4)  
- [x] **P2.5** Crate/bin rename `montanha-*` (optional) — status: `done` (not renaming; identity is documentation-first — `montanhadb.md`)  
- [x] **P2.6** Deterministic cluster simulation campaign (beyond unit tests) — status: `done` (RFC-0059 World swarm + invariants; `world_swarm_parallel_matches_serial`)

---

## 7. Status (living — update with every implementing PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Normative docs + link from montanhadb | done | in-tree + montanha-invariants-and-tests.md | 2026-08-12 |
| P0.2 | p0 | Multi-range puts | done | multi_range_puts_different_leaders | 2026-08-12 |
| P0.3 | p0 | Majority durability + orphan discard | done | majority_* / put_fails_* / heal_retry | 2026-08-12 |
| P0.4 | p0 | Strong / local reads | done | strong_read_refuses_deposed_and_dual_leader | 2026-08-12 |
| P0.5 | p0 | Range HA failover | done | range_failover_after_leader_loss | 2026-08-12 |
| P0.6 | p0 | DCS-on-store + heal/retry | done | dcs_on_store_* / dcs_create_* | 2026-08-12 |
| P0.7 | p0 | Deep tests + clippy gate | done | cargo test/clippy pedradb-store | 2026-08-12 |
| P0.8 | p0 | Invariant / test operator doc | done | docs/montanha-invariants-and-tests.md | 2026-08-12 |
| P1.1 | p1 | In-range multi-key atomic | done | put_batch + put_batch_* tests | 2026-08-12 |
| P1.2 | p1 | Live hub MVP | done | `subscribe_leadership` + failover stream test | 2026-08-24 |
| P1.3 | p1 | Cluster id / membership | done | `\0store/cluster/{id,membership}` + ClusterMismatch | 2026-08-24 |
| P1.4 | p1 | Multi-process/network harness | done | montanha-store-smoke + multiprocess_tx test | 2026-08-12 |
| P2.1 | p2 | Cross-range TX | done | commit_tx / tx_start / tx_finish 2PC | 2026-08-12 |
| P1.5 | p1 | Metrics or explicit none | done | `StoreMetrics` commits/NotCommitted/elections | 2026-08-24 |
| P1.6 | p1 | DCS leases fail-safe | done | F7 kernel + `dcs_lease_fail_safe_on_reopen` | 2026-08-24 |
| P2.2 | p2 | Placement/split/merge | done | `split_range_at` / `merge_adjacent_ranges` | 2026-08-24 |
| P2.3 | p2 | Prod binary/TLS/auth | done | RFC-0050/0021 montanha-tcp TLS | 2026-08-24 |
| P2.4 | p2 | etcd façade optional | done | `EtcdNeedFace` | 2026-08-24 |
| P2.5 | p2 | montanha-* rename | done | not renaming; docs-first identity | 2026-08-24 |
| P2.6 | p2 | Cluster simulation campaign | done | RFC-0059 World swarm | 2026-08-24 |

**P0 validation note (2026-08-12):** Re-ran RFC §8–§9 against in-tree `pedradb-store` / `pedradb-dcs`; map in [montanha-invariants-and-tests.md](../montanha-invariants-and-tests.md). P0.1–P0.8 marked `done` only after suite + adversarial rules held.

---

## 8. Verification matrix (validator checklist)

For each P0 slice, evidence must include command + exit 0 + assertion that the **invariant IDs** were exercised (test names in §9).

| Slice | Invariants | Minimum evidence |
|-------|------------|------------------|
| P0.1 | — | Links valid; no contradiction with §3 identity rules |
| P0.2 | I-MR-1, I-MR-2 | `store-multi-range` test log |
| P0.3 | I-MAJ-1..4 | majority + partition NotCommitted + orphan/heal tests |
| P0.4 | I-RD-1..3 | strong/dual-leader + doc naming |
| P0.5 | I-HA-1..3 | failover test log |
| P0.6 | I-DCS-1..5 | dcs replicate + heal/retry/put |
| P0.7 | all P0 I-* covered | full `cargo test -p pedradb-store --lib`; clippy store |
| P0.8 | — | doc maps test name → invariant ID |

**Adversarial rules (validator must try):**

1. Ok without majority → **fail product**.  
2. Strong success on deposed or dual leader → **fail**.  
3. NotCommitted then heal/tick installs lock without Ok → **fail**.  
4. NotCommitted then retry bricks puts → **fail**.  
5. Claiming linearizable for LocalApplied in docs → **fail**.  
6. Marketing “Montanha = etcd” or “= TiKV” in P0 docs → **fail**.

---

## 9. Deep test catalogue (normative names)

Tests **must** exist under `pedradb-store` (and `pedradb-dcs` where apply semantics matter). Names may match existing tests if semantics equal; if missing, implementer adds them.

### 9.1 Store / multi-Raft / durability

| Test id (logical) | Must prove | Maps to |
|-------------------|------------|---------|
| `T-MR-multi-range-puts` | ≥2 ranges, puts visible on all/majority peers | I-MR-*, I-MAJ-1 |
| `T-MAJ-three-peer-put` | After Ok put on N=3, applied on ≥2 (prefer 3 after catch-up) | I-MAJ-1 |
| `T-MAJ-partition-put-fails` | Only leader live → put **Err**, applied count 0 for that kv | I-MAJ-2 |
| `T-MAJ-orphan-discard` | After NotCommitted, last_index==commit on leader; heal+tick no silent apply | I-MAJ-3 |
| `T-MAJ-local-only-no-commit` | Hook or path that cannot majority-replicate does not advance commit | I-MAJ-2 |
| `T-RD-strong-vs-local` | Strong fails on follower; LocalApplied allowed | I-RD-* |
| `T-RD-dual-leader-fail-closed` | Two Role::Leader claims → Strong fails on **both**; range_leader none | I-RD-2 |
| `T-HA-leader-loss` | Partition leader out → elect → put → prior key on remaining peers | I-HA-* |
| `T-APPLY-pipeline-not-stuck` | Duplicate Create in log does not freeze applied&lt;commit; following put applies | I-MAJ-4 |

### 9.2 DCS-on-store

| Test id | Must prove | Maps to |
|---------|------------|---------|
| `T-DCS-create-replicated` | create Ok, peers see value, second create fails, cas renew works | I-DCS-1..4 |
| `T-DCS-partition-create-fails` | minority → NotCommitted; key absent everywhere | I-DCS-4, I-MAJ-2 |
| `T-DCS-heal-retry-put` | NotCommitted → heal → tick no lock → retry create Ok → put x/y majority | I-DCS-5, I-MAJ-4 |

### 9.3 Apply semantics (`pedradb-dcs`)

| Test id | Must prove |
|---------|------------|
| `T-DCS-APPLY-create-idempotent` | Second apply Create with key present does not error hard and does **not** overwrite; **client** pre-check still rejects create race; TTL re-create is Cas-on-corpse |
| `T-DCS-APPLY-cas-roundtrip` | encode/decode + cas path |

### 9.4 Quality gates

| Gate | Command / rule |
|------|----------------|
| Unit/integration | `cargo test -p pedradb-store --lib` exit 0 |
| DCS apply | `cargo test -p pedradb-dcs --lib` exit 0 when apply changed |
| Clippy | `cargo clippy -p pedradb-store --all-targets -- -D warnings` exit 0 |
| No `unsafe` in store | `#![forbid(unsafe_code)]` remains |
| Forbid silent identity drift | Docs review against §3.3 |

### 9.5 Depth requirements (what “deep” means here)

For P0, tests **must**:

1. Drive **public** store/DCS APIs (not only private hooks), except one explicitly named sim hook for local-only append if kept.  
2. Use **real PedraDB apply** on peers (no mock state machine replacing apply).  
3. Cover **failure** paths (partition, dual leader, NotCommitted, retry), not only happy path.  
4. Assert **peer-quorum observations** (`count_applied_eq` or per-node get), not only leader memory.  
5. Remain **deterministic** under in-process ticks (no flaky sleeps as sole sync).

P1+ may add multi-process and fault-injection network tests; P0 does not require TCP for store if in-process proves I-*.

---

## 10. Documentation requirements

### 10.1 Must exist for P0 `done`

| Doc | Content |
|-----|---------|
| **This RFC** | Status table accurate |
| `docs/montanhadb.md` | One-sentence product; link **RFC-0013 as contract**; FDB-shaped identity |
| `docs/montanha-vs-foundationdb.md` | Doctrine FDB vs plumbing |
| `docs/montanha-layering-dcs-on-store.md` | DCS on store rules |
| `docs/montanha-invariants-and-tests.md` (**new in P0.8**) | Table: invariant ID → test id → crate; how to run gates |

### 10.2 Documentation quality bar

- Consistency models **named** every time reads are described.  
- No claim of linearizable for local reads.  
- No claim of FDB/TiKV production parity.  
- Bootstrap raft path labeled **bootstrap**, not end state.

### 10.3 Screenshots

**None — backend / library product.** Explicitly N/A.

### 10.4 Telemetry / analytics

**P0:** none required (library correctness first).  
**P1.5:** counters or explicit deferral with reopen criteria.

---

## 11. Out of scope (this RFC)

| Out | Why |
|-----|-----|
| PedraDB kernel redesign | RFC-0001 |
| TiKV / FDB / etcd wire compatibility | Optional P2 façade only for etcd-class |
| Full distributed SI/2PC / SQL HTAP | Layers later; P2 strategy only |
| PD-scale scheduler | P2 |
| Renaming all crates to `montanha-*` | Optional P2.5 |
| Beating FDB/TiKV benchmarks | Non-goal |
| Montanha-Live as sole fencing | Forbidden forever |
| Multi-writer same PedraDB directory | Forbidden forever |
| Expanding bootstrap single-Raft into general data plane as brand | Forbidden |

---

## 12. Implementation constraints (precise, not a full design)

1. **L0:** PedraDB only via public APIs (`put`/`get`/`begin`/`apply` paths as needed).  
2. **L1 default plumbing:** range-partitioned Raft logs + apply to PedraDB; majority of **configured** membership.  
3. **Client Ok ⇒ majority commit** of that entry before return.  
4. **NotCommitted ⇒ discard** uncommitted client index on all peers (or equivalent that preserves I-MAJ-3/4).  
5. **DCS apply:** Create/Cas(rev=0) apply must not permanently stall the Raft apply cursor if key already exists (idempotent apply — **no overwrite**); **client** pre-check remains strict. Re-create after TTL is `Cas` on the physical revision (`bind_absent_create`), not Create overwrite.  
6. **Safe leader:** unique participating Leader role; else no Strong serve / no client propose.  
7. **Meta keys:** document a prefix (e.g. `m/`) so DCS keys land in stable ranges; do not require separate consensus domain for P0.  
8. **Forbid unsafe** in Montanha store crate.

---

## 13. Approval and change control

1. **Status `draft` → `approved`:** product owner explicitly accepts §3–§5 and P0 slices.  
2. **While `approved` / `in-progress`:** behavioral changes to §5 invariants require RFC edit in the **same PR** as code.  
3. **Marking slice `done`:** PR must list invariant IDs + test ids + command lines run.  
4. **Conflicts** with older montanha docs: **this RFC wins** after approval; update the older doc in the same PR.

---

## 14. Suggested validation script (human or agent)

```bash
# P0.7 gates
cargo test -p pedradb-store --lib
cargo test -p pedradb-dcs --lib
cargo clippy -p pedradb-store --all-targets -- -D warnings

# Spot-check named suites (adjust filters to real test names)
cargo test -p pedradb-store --lib multi_range
cargo test -p pedradb-store --lib majority
cargo test -p pedradb-store --lib put_fails
cargo test -p pedradb-store --lib strong_read
cargo test -p pedradb-store --lib failover
cargo test -p pedradb-store --lib dcs_
```

Validator then walks §8 adversarial rules and §5 invariant list with **pass/fail notes**.

---

## 15. North-star sentence (repeat)

**MontanhaDb is FoundationDB-shaped HA on PedraDB: a correct distributed ordered KV substrate, layers for coordination and higher models, majority durability and named consistency by construction — coordination as data, replication as plumbing, never etcd-as-identity, never TiKV-as-brand.**
