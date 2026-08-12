# RFC-0015: Audit-driven correctness fixes (durability, seams, supply chain)

**Status:** done (P0–P2)  
**Updated:** 2026-08-12  
**Parent:** [RFC-0001](0001-pedradb-high-level-spec.md)  
**Extends:** [RFC-0005](0005-p0-durability-and-usage.md) (durability contract), [RFC-0011](0011-env-fault-injection.md) (Env seam)  
**Complements:** [RFC-0014](0014-rocks-pebble-redwood-maturity.md) (maturity shape — **not** a substitute for this correctness wave)  
**Source audit:** `/audit-pedradb` session 2026-08-12 (collector `.grok/skills/audit-pedradb/`; gates: core 118, store 42, sim 22, dst 5 passed)

---

## Background

`/audit-pedradb` (inspired by caixote `audit-metal`, retargeted to storage) scored the workspace **7.5/10**: strong default fsync-before-Ok, multi-key atomic `WriteRecord`, WAL CRC/orphan fail-closed, flush tmp+rename + MANIFEST-before-WAL-rotate (F1/F21), Montanha majority/dual-leader tests, Env/Host seams on the **kernel** path.

**No CRITICAL** violations of the form “`Ok` without required WAL fsync” or “half multi-key TX visible after reopen” were found on the happy path.

Several **HIGH** residual classes remain: post-failed-sync WAL/mem divergence, discarded `sync_dir`, `DirLock` Drop bypassing `Env`, and upper-layer `std::fs` (raft persist, WAL ship). **MEDIUM** gaps: unbounded `range`, no RustSec CI, coarse `ConcurrentDb` write lock across fsync, intentional swallow of auto-compact after flush without ops signal.

This RFC is the **fix plan** for those findings. It does **not** claim Rocks/Pebble/FDB field maturity ([`robustness-vs-rocks-pebble-fdb.md`](../robustness-vs-rocks-pebble-fdb.md)).

### Audit scorecard (source of truth for “what to fix”)

| Sev | ID | Finding | Where |
|-----|-----|---------|--------|
| — | C0 | No critical Ok-without-fsync / half-TX on audited paths | `commit_ops_with`, TX, flush F1/F21 |
| HIGH | H1 | After `append` OK + `sync_all` Err: WAL may hold record; mem not applied; later fsync can make “failed” write durable while in-process `get` misses until reopen | `db.rs` `commit_ops_with` |
| HIGH | H2 | `sync_dir` errors discarded (`let _ =`) on flush / MANIFEST / checkpoint | `db.rs`, `manifest.rs` |
| HIGH | H3 | `DirLock::Drop` uses `std::fs` — unlock not fault-injectable | `lock.rs` |
| HIGH | H4 | `pedradb-raft` persist uses raw `std::fs` + best-effort dir sync discard | `raft/persist.rs` |
| HIGH | H5 | `pedradb-replicate` WAL ship uses raw `std::fs` + best-effort dir sync discard | `replicate/src/lib.rs` |
| MED | M1 | Unbounded `Db::range` still a public OOM footgun (`range_limited` / `scan` exist) | `db.rs` |
| MED | M2 | `ConcurrentDb` holds write lock across put+fsync (correct but high contention) | `concurrent.rs` |
| MED | M3 | No `cargo deny` / `cargo audit` in CI | workspace |
| MED | M4 | Auto-compact failure after successful flush is swallowed without ops signal | `db.rs` `flush` |
| MED | M5 | Raft multi-process tests / net use wall `thread::sleep` (OK for integration; keep out of store control plane) | `raft` — **document only** |
| NOTE | N1 | RFC-0014 P1 streaming/lazy blocks already tracked — **not** re-sliced here | RFC-0014 |
| NOTE | N2 | Montanha I-MAJ / I-RD / I-HA / I-DCS tests already green — **keep as regression gates** | `pedradb-store` |

---

## Problems this solves

- **Problem (H1):** Client receives `Err` on durability failure but process memory and future recover can disagree; later successful writes may fsync unacked WAL prefix.  
- **Problem (H2):** Rename of SST / MANIFEST / `CURRENT` may not be power-loss durable when dir fsync fails silently.  
- **Problem (H3–H5):** Fault injection and DST cannot see lock release, raft meta I/O, or replica WAL ship.  
- **Problem (M1/M3/M4):** Production footguns and supply-chain blindness outside the LEDGER hunt.  
- **Problem:** Findings live only in a chat report — no living delivery map.

---

## Proposed solution

### H1 — Failed sync after append (kernel)

Choose a **single documented policy** (implement P0.1):

**Preferred (fence):** if `append_record` succeeded and required `sync_all` fails, mark the `Db` **I/O-fenced** (or return a fatal durability error type) so further writes/reads that depend on mem coherence refuse until `close` + `open` (recover rebuilds mem from WAL). Optionally attempt best-effort `sync` retry once.

**Alternative (reconcile):** on sync failure, still do not apply mem; next open recovers; in-process must not silently continue — same fence.

**Not acceptable:** continue accepting puts after failed fsync without fence or explicit `WriteOptions`/API for “uncertain”.

Document in `docs/usage.md` + rustdoc: **Err on sync does not mean “record absent on disk”** (uncertain outcome); **Ok still means durable** when `sync=true`.

### H2 — `sync_dir` when durability required

When `OpenOptions.sync` is true (or the write path requested sync), **propagate** `Env::sync_dir` failures on:

- flush after SST rename  
- MANIFEST/`CURRENT` install  
- checkpoint end  

When `sync=false`, best-effort discard remains OK (document).

### H3 — DirLock and Env

Hold enough context to release via `Env` on Drop **or** document that Drop is OS-best-effort only and provide `DirLock::release(env)` used by `Db::close` / drop path with Env. Prefer: store `path` + release through a process-global is wrong; use `Option` cleanup in `Db::drop` via `env` before dropping lock, and make `DirLock::drop` only remove if still held (still may need std for pure Drop — acceptable if **primary** unlock path uses Env).

Minimum bar: **acquire/write LOCK stays on Env** (already true); **explicit `Db` shutdown** unlocks via Env; Drop remains best-effort std with comment + audit note.

### H4 / H5 — Upper layers on Env

- Raft persist: write/read log+meta through `Env` (or thin `PersistEnv` alias to core `Env`).  
- Replicate ship/append: same.  
- Keep production default `StdEnv`. Wire `FailingEnv` tests for “nth op fails mid atomic_write”.

### M1 — Range API

- Document that `range` is convenience / small-DB; prod pagination uses `range_limited` / `scan`.  
- Add clippy-allowed or rustdoc `# Panics`/risk note; optional `OpenOptions` / feature later to deny unbounded (P2 only if needed).

### M2 — ConcurrentDb

- No API change required for correctness.  
- Document contention: write lock covers WAL fsync; not Rocks multi-writer.

### M3 — Supply chain

- Add `deny.toml` (advisories) + CI job or documented scheduled `cargo audit`.  
- Ignore policy only with justification comments.

### M4 — Compact lag signal

- On auto-compact failure after flush: increment `DbStats` counter or last-error slot (no fail of flush).  
- Optional tracing event.

### M5 — Wall sleep

- Out of fix code for store; note in this RFC only. Raft integration tests may keep sleep.

---

## Delivery slices

### P0 — durability correctness (kernel; shippable alone)

Closes H1 + H2 with tests. No raft/replicate work required for value.

- [x] **P0.1** Fence (or equivalent hard policy) after required `sync_all` failure post-append; document uncertain-outcome vs Ok contract — status: `done`  
- [x] **P0.2** Propagate `sync_dir` when `sync=true` on flush SST publish, MANIFEST install, checkpoint — status: `done`  
- [x] **P0.3** Unit + sim tests: SyncFail after append → no further put without reopen; recover shows consistent policy; SyncFail on `sync_dir` fails flush/MANIFEST when sync — status: `done`  
- [x] **P0.4** `docs/usage.md` + rustdoc durability table updated for fence + dir sync — status: `done`  

### P1 — Env completeness (seams)

Closes H3–H5 and extends RFC-0011.

- [x] **P1.1** DirLock release path observable via Env (shutdown/unlock); Drop documented — status: `done`  
- [x] **P1.2** `pedradb-raft` persist atomic write/load via `Env` — status: `done`  
- [x] **P1.3** `pedradb-replicate` ship/append via `Env` — status: `done`  
- [x] **P1.4** `FailingEnv` / sim tests for raft persist + replicate nth-op and dir sync — status: `done`  

### P2 — ops / supply chain / API hygiene

Closes M1–M4 (M5 doc-only in P2.0).

- [x] **P2.0** Document M5 (raft wall sleep) + ConcurrentDb contention (M2) in usage/architecture — status: `done`  
- [x] **P2.1** Unbounded `range` risk docs + examples prefer `range_limited`/`scan` (M1) — status: `done`  
- [x] **P2.2** Auto-compact failure counter / last error on `DbStats` (M4) — status: `done`  
- [x] **P2.3** `deny.toml` + CI `cargo deny check advisories` (or scheduled audit) (M3) — status: `done`  

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Fence after failed required WAL sync | done | in-tree | 2026-08-12 |
| P0.2 | p0 | Propagate sync_dir when sync=true | done | in-tree | 2026-08-12 |
| P0.3 | p0 | Tests: SyncFail append + sync_dir | done | sim + core | 2026-08-12 |
| P0.4 | p0 | Usage/rustdoc durability update | done | usage.md + db rustdoc | 2026-08-12 |
| P1.1 | p1 | DirLock Env-visible unlock | done | lock + Db Drop/close | 2026-08-12 |
| P1.2 | p1 | Raft persist on Env | done | persist `*_on` | 2026-08-12 |
| P1.3 | p1 | Replicate ship on Env | done | append/pull `*_on` | 2026-08-12 |
| P1.4 | p1 | Sim tests raft+replicate Env | done | FailingEnv tests | 2026-08-12 |
| P2.0 | p2 | Doc M2/M5 contention & wall sleep | done | usage.md + concurrent rustdoc | 2026-08-12 |
| P2.1 | p2 | Range API footgun docs/examples | done | usage.md + range rustdoc | 2026-08-12 |
| P2.2 | p2 | Auto-compact failure stats | done | DbStats + unit test | 2026-08-12 |
| P2.3 | p2 | cargo deny / audit CI | done | deny.toml + supply-chain.yml | 2026-08-12 |

---

## Acceptance criteria

### Tests

**P0**

- [x] `FailingEnv` / `FaultKind::SyncFail` (or equivalent): after put/commit gets durability Err, further `put` returns fenced/fatal until reopen.  
- [x] Reopen after fenced sync failure: state matches policy (recovered WAL prefix applied; no silent half-TX).  
- [x] Multi-key TX: same fence rules as put.  
- [x] `sync_dir` failure during flush with `sync=true` → `Err`; no MANIFEST pointing at incomplete publish; reopen consistent (prior version or fail-stop, never half-visible TX).  
- [x] Existing gates still green: `cargo test -p pedradb-core --lib`, `pedradb-sim`, `pedradb-dst` denser silent_wrong, store majority suite.

**P1**

- [x] Raft persist: fail_after during atomic_write does not leave torn final meta without error; recover/load fail-closed or prior version.  
- [x] Replicate append: sync/dir failure surfaces as ship error; primary contract unchanged.  
- [x] DirLock: unlock path exercised under FailingEnv without requiring Drop std alone for the success path.

**P2**

- [x] `DbStats` (or equivalent) reflects compact auto-fail at least once in a unit test.  
- [x] CI config exists for advisories (or docs explain manual schedule + owner).  
- [x] usage examples for large scans use limited/streaming APIs.

### Telemetry / analytics

- None external. In-process: compact-fail counter (P2.2). Optional `tracing` on fence and sync_dir fail (P0).

### Documentation

- This RFC status table updated per slice.  
- `docs/usage.md` durability section (P0.4).  
- `docs/dst-seams.md` row for raft/replicate Env when P1 lands.  
- `docs/open-items.md` next-action points at this RFC until P0 done.  
- Skill `audit-pedradb` may reference RFC-0015 as the fix backlog (optional follow-up).

### Screenshots

- backend-only.

---

## Out of scope

- Claiming field parity with RocksDB / Pebble / FoundationDB simulation.  
- Multi-writer OCC / write-amp (RFC-0014 P2 / open items).  
- Streaming range / lazy SST blocks (**RFC-0014 P1** owns those).  
- Replacing raft integration `thread::sleep` with a full discrete-event runtime (store Queued path is enough for DST).  
- Lying fsync that returns Ok but drops bytes (RecordingEnv / external LD_PRELOAD — RFC-0011 residual).  
- Changing Montanha majority algorithm (only keep regression tests green).

---

## Design notes (non-binding implementation hints)

### Fence sketch

```text
commit_ops_with:
  append_record(rec)?
  if do_sync:
    match sync_all():
      Err(e) => { self.durability_fenced = true; return Err(e) }
  apply_record(mem, rec)
  Ok(())

put/commit/flush:
  if durability_fenced { return Err(CoreError::DurabilityFenced) }
```

Reopen clears fence by construction (new `Db`).

### sync_dir

```text
if sync {
  env.sync_dir(dir)?;  // not let _ =
}
```

### Finding → slice map

| Finding | Slice |
|---------|-------|
| H1 | P0.1, P0.3, P0.4 |
| H2 | P0.2, P0.3, P0.4 |
| H3 | P1.1 |
| H4 | P1.2, P1.4 |
| H5 | P1.3, P1.4 |
| M1 | P2.1 |
| M2 | P2.0 |
| M3 | P2.3 |
| M4 | P2.2 |
| M5 | P2.0 |

---

## Relationship to continuous hunt

LEDGER / dense sweeps (`pedradb-dst`, out-of-tree `determinismo`) remain the silent-wrong net. This RFC closes **structural** gaps the 2026-08-12 audit found so hunts are not papering over known WAL/mem or seam holes.

When a slice ships: mark checkbox + Status row `done` **in the same PR** as the code (RFC skill rule).
