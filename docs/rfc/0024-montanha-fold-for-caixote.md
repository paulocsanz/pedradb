# RFC-0024: Montanha Fold — Slipstream-class materializer for Caixote

**Status:** done
**Updated:** 2026-08-14
**Parents:** [RFC-0019](0019-local-primitive-for-platform-and-scylla-need.md) (CAS / seq / CHANGELOG) · [RFC-0017](0017-montanha-fdb-class-substrate.md) (Montanha SoR) · [RFC-0021](0021-montanha-fdb-tikv-parity-gaps.md)
**Research:** [`../slipstream-and-quicksilver-learnings.md`](../slipstream-and-quicksilver-learnings.md) §10 · [`../../caixote/docs/research/quicksilver-slipstream-orchestration-scale.md`](../../../caixote/docs/research/quicksilver-slipstream-orchestration-scale.md)
**First consumer:** Caixote (`caixote-api`, federation-api, procurador)

---

## 0. Honesty

| Claim that is **false** | Truth |
|-------------------------|--------|
| “This RFC makes Montanha as fast as Quicksilver on writes” | Writes stay Raft+fsync. QS scale is **reads that never hit the root**. |
| “Every Caixote host becomes a Montanha voter” | **Forbidden.** Fold consumers are not Raft members. |
| “Fold `get` is linearizable” | Fold is **LocalApplied** at a pinned seq. Use `get_strong` on Montanha for SoR. |
| “We vendor `beyond-slipstream` / NATS JetStream” | **No.** Same *discipline*, our log (Montanha/Pedra). Jepsen: JetStream ack ≠ durable. |
| “P0 replaces federation PG + 5 s dump” | P0 is the fold **kernel**. P1.3: 5 s is the tick; reopen with `observed-fold.json` is a delta. |

P0 is useful **without** Caixote: a process can fold a Montanha prefix, crash, and resume from a cursor that only names applied revisions.

---

## Background

Caixote federation today: PostgreSQL SoR, each `caixote-api` sends a **full** state snapshot every 5 s over gRPC. That is O(state) on the wire. Desired/observed live in SQL; procurador asks the control plane on the request path.

Montanha is the lab SoR we want for desired state, leases, assignments, CAS (RFC-0017/0021). Pedra already has CHANGELOG + seq pin (RFC-0019). `JournalConsumer` still advances the pin on **read**, not after the caller applies — the Slipstream footgun.

Quicksilver v2 and Slipstream show the missing product: a **fold**. Leaves materialize a prefix, serve locally (µs–ms), resume from a sequence. They are **not** a second transactional DB.

Lab numbers (`findings/perf-gate-v0-verify`): Montanha put p50 ~246 ms (lab, not field); get LocalApplied p50 ~0.07 ms. The fold must stay on the get side.

---

## Problems This Solves

- **Problem:** A Caixote host or procurador cannot restart without a full re-list (federation dump / NATS replay / scan-then-watch race).
- **Problem:** Persisting a watch pin on *receipt* silently skips updates after crash (`JournalConsumer::catch_up`).
- **Problem:** Putting every host on the Montanha Raft (or `get_strong` on every DNS/proxy lookup) cannot reach request-path scale.
- **Problem:** After WAL rotate / log compact, `WalRotated` has no “expired cursor → resync” protocol.
- **Problem:** Federation’s 5 s full snapshot will not survive host×VM growth even if Montanha is the SoR.

---

## Proposed Solution

A **fold layer** (`pedradb-fold`) on Montanha:

```
Caixote API / IaC
    → Montanha (SoR): desired, lease, assignment, CAS, 2PC
         log (seq / raft index)
    → Fold on caixote-api / procurador:
         watch_applied → Pedra local prefix
         get / range = LocalApplied at pinned cursor
         not a Raft member
```

- **Cursor-after-apply:** persisted cursor `C` ⇒ `apply()` returned for every revision ≤ `C`.
- **State-sync then tail:** no-cursor start is last-per-key, then live. Never scan-then-watch.
- **Prefix:** a host fold is `/host/{id}/` plus the VM keys it owns — not the cluster.
- **SoR stays Montanha.** Fold is a cache of the log. Delete the fold, resume from cursor (or artifact in P2).
- **Reads on the request path never call `get_strong`.**

P0 ships the combinator + Pedra-backed fold + in-process Montanha follow. P1 wires Caixote (delta sync, local route read). P2 is bootstrap/cache roles.

---

## Delivery slices

### P0 — fold kernel (useful without Caixote)

- [x] **P0.1** `watch_applied` combinator: batch → apply → *then* persist pin — status: `done`
- [x] **P0.2** `FoldStore` trait + Pedra backend: atomic `apply(batch, cursor)`, `load`, `get`, `range` — status: `done`
- [x] **P0.3** In-process follow of a Montanha prefix from seq (CHANGELOG / applied log) — status: `done`
- [x] **P0.4** Crash/resume + Caixote-shaped prefix fixture (host + two VM keys) — status: `done`

### P1 — watch on the wire + first Caixote consumer

- [x] **P1.1** Networked state-sync watch then tail (no scan-then-watch) — status: `done`
- [x] **P1.2** `CursorExpired` + synthetic deletes (key-only diff) then re-list — status: `done`
- [x] **P1.3** Caixote federation: intent/observed **by seq** (5 s dump becomes backstop) — status: `done`
- [x] **P1.4** One read path (procurador **or** `caixote-api` desired) served from the local fold — status: `done`

### P2 — bootstrap and edge cache

- [x] **P2.1** Fold export/import (checkpoint + cursor + hash + verify-by-reopen) — status: `done`
- [x] **P2.2** Logical ship cursor: WAL rotate = `CursorExpired`, not “copy SSTs” — status: `done`
- [x] **P2.3** Named roles `storage` / `relay` / `proxy` (cache; keep keys / evict values) — status: `done`

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | watch_applied combinator | done | `pedradb-fold` | 2026-08-14 |
| P0.2 | p0 | FoldStore + Pedra backend | done | `pedradb-fold` | 2026-08-14 |
| P0.3 | p0 | In-process Montanha prefix follow | done | `pedradb-fold` | 2026-08-14 |
| P0.4 | p0 | Crash/resume + host/VM fixture | done | `pedradb-fold` | 2026-08-14 |
| P1.1 | p1 | Networked state-sync watch | done | `pedradb-fold` | 2026-08-14 |
| P1.2 | p1 | CursorExpired + synthetic deletes | done | `pedradb-fold` | 2026-08-14 |
| P1.3 | p1 | Federation intent/observed by seq | done | `seq_sync` + live `observed-fold.json` | 2026-08-14 |
| P1.4 | p1 | One Caixote read path from fold | done | `procurador` fold_read | 2026-08-14 |
| P2.1 | p2 | Export/import artifact | done | `pedradb-fold` | 2026-08-14 |
| P2.2 | p2 | Logical ship cursor | done | `pedradb-fold` | 2026-08-14 |
| P2.3 | p2 | storage/relay/proxy roles | done | `pedradb-fold` | 2026-08-14 |

---

## Acceptance Criteria

### Tests

**P0**

- `watch_applied_pin_after_apply` — crash between recv and apply; resume does not skip.
- `fold_apply_cursor_atomic` — torn apply cannot leave cursor ahead of data.
- `fold_follow_montanha_prefix` — only in-prefix keys appear; other ranges ignored.
- `fold_resume_after_reopen` — host/VM fixture: restart delivers only seq > pin.
- FailingEnv: transient `FoldStore::apply` re-queues; does not advance pin (F47 class).

**P1**

- `watch_state_sync_then_tail` — no-cursor start is last-per-key then live.
- `watch_seed_then_watch_gap_closed` — write between list and subscribe is visible.
- `cursor_expired_synthetic_deletes` — keys deleted in the gap vanish; recreate order is delete-then-put.
- Caixote (when wired): host kill + reopen does not require a full resource dump to converge desired.

**P2**

- Export verify-by-reopen: recovered cursor == live cursor.
- `WalRotated` / compact past pin → `CursorExpired`, not a stuck shipper.

### Telemetry / Analytics

P0: counters `fold_apply_batches`, `fold_cursor`, `fold_apply_err` (in-process).
P1: `fold_watch_resync`, `fold_cursor_expired`, federation `sync_delta_keys` vs `sync_full_dump`.
P2: cache hit/miss if proxy role exists.
Working-set access log is **not** P0 (measure before P2.3).

### Documentation

- This RFC status table updated in the same change as code.
- `docs/usage.md`: how to open a fold, pin, resume.
- Caixote: when P1.3 lands, federation ARCHITECTURE “state sync every 5 s” becomes “delta by seq; 5 s is backstop.”

### Screenshots

Backend-only.

---

## Out of scope

- Replacing Montanha with NATS / `beyond-slipstream`.
- Making fold gets linearizable or adding voters per Caixote host.
- Quicksilver L2 sharded cache / reactive prefetch (after a measured working set; not this RFC’s P0).
- Sleep/wake of VMs (Caixote density; not the fold).
- Elastic disk / GlideFS (RFC 0091 on the Caixote side).
- Unbundling Raft log vs storage (RFC-0021 still A).
- Drop-in JetStream or QS wire protocol.

---

## Key layout (Caixote fixture — convention, not a new engine)

```
/vm/{id}                 desired generation + spec hash
/assign/{vm}             host + claim_expires
/host/{id}/cap
/host/{id}/observed/{vm}   # P1: seq-stamped, not a 5 s blob
/route/{svc}/{port}      procurador fold (P1.4)
```

A host fold watches `/host/{id}/` plus the `/vm/*` and `/assign/*` keys it owns. Procurador fold watches `/route/` in its DC (P1.4 can start with one prefix).

---

## Mapping to prior notes

| Note | What this RFC takes |
|------|---------------------|
| Slipstream `watch_applied` | P0.1 invariant |
| QS v2 “don’t put full data on every leaf” | Fold is not a Raft replica |
| Caixote 5 s dump | P1.3, not P0 |
| RFC-0019 CHANGELOG | SoR of the feed; fold is the cache |
| F47 | Transient apply must not advance pin; same class |
