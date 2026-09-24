---
name: audit-pedradb
description: >
  Audit PedraDB / Montanha for storage-engine correctness: durability contracts
  (WAL fsync before Ok), integrity (CRC fail-closed, never silent-wrong), TX
  all-or-nothing, Env/Host/Clock/Rng seams, recovery/torn-write, layer boundaries,
  concurrency, Montanha majority/leadership invariants, and DST determinism.
  Use when the user runs /audit-pedradb, asks to "audit pedradb", "audit core",
  "audit montanha", "silent wrong", "durability audit", or "env seam audit".
argument-hint: "[--fix] [crate-or-path...]"
---

# PedraDB / Montanha Storage Audit

Comprehensive audit for the PedraDB kernel and Montanha layers: **correctness
over thruput**. Inspired by caixote `audit-metal` (collect → investigate →
report), retargeted from async/executor safety to **LSM durability, integrity,
and deterministic fault seams**.

## Data collection (MANDATORY FIRST STEP)

Run from the **pedradb repo root**:

```bash
.grok/skills/audit-pedradb/collect-audit-data.sh
# optional scope:
.grok/skills/audit-pedradb/collect-audit-data.sh crates/pedradb-core crates/pedradb-store
```

Claude Code mirror (if present):

```bash
.claude/skills/audit-pedradb/collect-audit-data.sh
```

Analyze script output first. Do **not** re-run every `rg` by hand unless a
section needs deeper manual review.

### Script output map

| Section | What to hunt |
|---------|----------------|
| **DURABILITY CONTRACT** | `sync`, `sync_data`/`sync_all`/`sync_dir`, WriteOptions |
| **INTEGRITY / CHECKSUMS** | CRC, verify, corrupt paths fail-closed |
| **ENV / HOST SEAM BYPASS** | `std::fs` outside `StdEnv`, wall clock, OS RNG, sleep |
| **UNSAFE / FORBID** | `unsafe` vs `#![forbid(unsafe_code)]` |
| **RECOVERY / TORN WRITE** | WAL fragment orphans, MANIFEST/tmp+rename |
| **TX ATOMICITY** | single WriteRecord multi-key, commit path |
| **CONCURRENCY / LOCKS** | Mutex/RwLock inventory, ConcurrentDb |
| **FAIL-CLOSED / SILENT WRONG** | unwrap, default-on-error, swallow |
| **SWALLOWED ERRORS** | `let _`, `if let Err`, empty match arms |
| **LAYER BOUNDARIES** | core must not import upper crates |
| **DST / DETERMINISM** | Env/Clock/Rng/Host, RpcMode Queued |
| **MONTANHA STORE** | majority, NotCommitted, dual-leader, Strong |
| **BOUNDS / OOM** | unbounded range/collect, O(1) dynamic alloc budget |
| **SUPPLY CHAIN / CI** | cargo audit/deny |
| **LSM BISIMULATION** | α(LSM) → Map(K→V), snapshot isolation, shadowing |
| **CRASH REFINEMENT** | σ_rec = Prefix(AckedTxns), zero phantom records |
| **SPEC ANTI-VACUITY** | ∀ universal theorems in Lean, satisfiable hypotheses |
| **MIRI DATA-RACES** | Tree-Borrows, data race detection, zero UB |

Then complete the **manual** sections that greps cannot finish: lock scope
correctness, commit/fsync ordering, recover behavior under partial records,
majority Ok semantics.

## Why this matters

Storage bugs are **silent wrong** until production:

| Failure class | User-visible damage |
|---------------|---------------------|
| Ack without durable WAL | Data loss after crash |
| Half multi-key TX durable | Secondary index / FK corruption |
| CRC ignored / len not in checksum | Silent bitrot |
| Torn WAL accepted as full TX | Partial apply |
| `std::fs` bypassing `Env` | Fault injection blind; DST lies |
| Wall clock / OS RNG in engine | Non-reproducible hunts |
| Strong read under dual leader | Split-brain client sees fiction |
| Majority Ok without majority | Lost acknowledged puts |

**Doctrine (repo):** lab + reliability culture; never claim field maturity
Rocks/Pebble/FDB. Prefer fail-stop over silent wrong. Canonical robustness
narrative: `docs/robustness-vs-rocks-pebble-fdb.md`. Hunt ledger (if present):
`../determinismo/pedradb-dst/findings/LEDGER.md`.

## Audit scope

| Layer | Crates | Focus |
|-------|--------|--------|
| Kernel | `pedradb-core` | WAL, MemTable, SST, MANIFEST, TX, Env/Host, checkpoint/verify |
| Fault seams | `pedradb-sim`, `pedradb-dst` | FailingEnv, RecordingEnv, seed trials |
| Oracle | `pedradb-oracle` | model / optional live Rocks — never linked into core |
| Apply / Raft / Store | `pedradb-apply`, `pedradb-raft`, `pedradb-store` | ordered apply, multi-Raft, majority |
| Products | `pedradb-dcs`, `http`, `sql`, `stream`, `replicate` | must not break kernel contracts |

Default full workspace. Args to the collect script narrow paths.

## Architecture invariants

```
  App / raft / store / dcs / http / sql / stream
        │ embeds
        ▼
  pedradb-core  ──disk──► Env (StdEnv | FailingEnv | IoUringEnv)
                ──time──► Clock
                ──rng───► Rng
                ──bundle► Host / DetHost
```

**Rules:**

1. **`pedradb-core` is the local ACID ordered-KV kernel.** Upper crates embed it; they do not reimplement WAL/TX.
2. **All durable I/O goes through `Env` / `EnvFile`.** Only `StdEnv` (and env impl crates like `pedradb-io-uring`) call `std::fs` for engine data paths.
3. **Time and entropy for logic go through `Clock` / `Rng` (or `Host`).** No `SystemTime` / `thread_rng` in recovery, lease, or election logic that DST must replay.
4. **Core never imports** `pedradb-raft`, `pedradb-store`, `pedradb-dcs`, `pedradb-http`, `pedradb-sql`, `pedradb-stream`, `pedradb-replicate`, `pedradb-apply`, `pedradb-dst`.
5. **RocksDB is oracle-only** (`pedradb-oracle` feature) — never a runtime dep of core.
6. **`#![forbid(unsafe_code)]` on core** (and prefer on pure-logic crates). New `unsafe` needs an explicit exception audit.
7. **DST campaigns live out-of-tree** (`../determinismo`); this repo ships **seams only** — see `docs/dst-seams.md`.

---

## CRITICAL violations

### 1. Durability contract breach

**Contract (default):** after successful `put` / TX `commit` with sync enabled, WAL (and required dir entries) are durable — process crash must not lose the ack.

From script: `OpenOptions`, `sync_*`, WriteOptions.

**Manual review (MANDATORY for each write path):**

1. Does `Ok(())` return only **after** `sync_data` / `sync_all` (when sync=true)?
2. Is multi-key TX one atomic WAL record (not N independent records with partial Ok)?
3. Does flush/compact use **tmp + rename + sync_dir** so crash cannot leave half-SST referenced by MANIFEST?
4. On MANIFEST update failure after SST write, is there rollback / fail-stop (not "SST orphan silently used")?

**Violations:**

```rust
// BAD: return Ok before fsync when sync required
self.wal.write_record(&rec)?;
// missing file.sync_data()?
Ok(())

// BAD: multi-key as separate durable records with intermediate Ok visibility
for (k, v) in ops {
    self.wal.append_one(k, v)?;
    self.wal.sync()?; // partial TX durable — silent wrong for indexes
}
```

**Fix shape:** single `WriteRecord` for TX; fsync before Ok when sync; atomic publish via rename + dir sync.

### 2. Integrity: silent wrong on corrupt data

Checksum / CRC failures must **fail-closed** (error / refuse open), never skip and continue with wrong bytes.

Flag:

- CRC computed without covering length (classic truncation/extension bug)
- `verify_checksums` paths that ignore errors
- SST/WAL/MANIFEST readers that `continue` past corrupt blocks without bound/policy

**Anti-pattern:** "best-effort recover by skipping corrupt records" without an explicit, tested policy and metrics — defaults to silent data loss.

### 3. Env / Host seam bypass

In **engine / store data paths**, flag:

| Pattern | Severity | Notes |
|---------|----------|-------|
| `std::fs::*` in `pedradb-core` outside `env.rs` StdEnv impl | **CRITICAL** | Breaks FailingEnv |
| `File::open` / `OpenOptions` outside Env impls | **CRITICAL** | Same |
| `SystemTime` / `Instant::now` in election, lease, GC cutoff logic | **HIGH** | Breaks ManualClock replay |
| `thread_rng` / `OsRng` for election jitter | **HIGH** | Use `Rng` / `SeedRng` |
| `thread::sleep` in store/raft **production** paths | **HIGH** | Prefer logical `advance_time` / host clock |

Tests and CLI temp-dir cleanup may use `std::fs::remove_dir_all` — mark **OK if test-only**.

Reference: `docs/dst-seams.md`.

### 4. `unsafe` in forbidden crates

If `#![forbid(unsafe_code)]` is present, any `unsafe` is a compile failure — still report if someone `allow`s it.

If a crate drops forbid without justification (esp. core), **CRITICAL** process violation.

---

## HIGH severity

### 5. Recovery / torn write

WAL fragment rules (engine lore — verify still true in code):

- Torn tail discarded, not half-applied
- Orphan `Middle`/`Last` without valid `First` → fail-stop or skip policy **documented and tested**
- Empty/torn empty WAL does not brick open incorrectly

MANIFEST / SST:

- Publish only after durable SST
- CURRENT pointer updates atomic / recoverable
- No MANIFEST entry pointing at missing SST after crash mid-flush

**Manual:** read `wal/reader.rs` recover loop and flush/MANIFEST update in `db.rs` / `manifest.rs`. For each path: crash between every I/O step → state is empty, consistent prior, or fail-stop — never half-TX visible.

### 6. Transaction all-or-nothing

For multi-key TX:

- [ ] Ops buffered then one commit record
- [ ] Uncommitted TX not visible after reopen
- [ ] Commit Ok ⇒ all keys visible at commit seq; abort ⇒ none
- [ ] Snapshots do not see partial commit

Tests to expect (names evolve — search store/core tests): multi-key commit, crash after Ok, uncommitted discard.

### 7. Concurrency and lock correctness

`ConcurrentDb` and any `Mutex`/`RwLock`/`parking_lot`:

Build inventory from script (`mutex_fields`, `lock_acq_full`).

For **each** lock:

| Question | Bad answer |
|----------|------------|
| What invariant? | "misc state" |
| Hold across I/O (WAL sync, SST write)? | Yes without need |
| Release-reacquire same logical op? | TOCTOU on memtable/version |
| Check-then-act (`contains` then insert)? | Lost update |
| Multiple locks: order documented? | AB vs BA deadlock |

PedraDB is largely **single-writer** kernel + optional concurrent wrapper — do not invent multi-writer without RFC. Flag code that pretends multi-writer safety without seq/version rules.

**Note:** sync `std::sync::Mutex` / `parking_lot` is **fine** in this mostly-sync engine (unlike async metal). Flag only: long holds during disk I/O that block unrelated readers without need, or incorrect critical sections.

### 8. Swallowed errors (investigate every hit)

Same discipline as audit-metal Section 6 — **every** `let _`, named `let _foo`, `.ok();`, empty `Err(_) => {}`, log-and-continue on durability paths.

**Especially CRITICAL when discarded type is:**

- WAL/SST/MANIFEST write or sync `Result`
- Channel send of commit/apply
- Lock / dir lock release paths that surface errors
- Raft/store majority path errors turned into success

Template per hit:

```markdown
### Discard: `{file}:{line}`
**Code:** …
**Discarded type:** …
**On failure today:** …
**If propagated:** …
**Verdict:** CRITICAL / HIGH / OK-with-comment
```

Named discards (`let _result =`) are **worse** than `let _ =`.

### 9. Layer boundary violations

From script `core_must_not_import_upper` — any hit is **HIGH**.

Also flag:

- Public core API leaking sim-only types without feature gate
- Store reimplementing local TX instead of `pedradb_core::Db`
- HTTP/SQL mapping that acknowledges before kernel Ok

### 10. DST / determinism regressions

When changing kernel or store:

- Prefer `open_with_env` / `open_with_host` / `open_with_rng` over hard-wired Std*
- Store RPC: `RpcMode::Queued` + `PeerMsg` must remain injectable (`drain_outbound` / `handle_inbound`)
- No new wall sleeps in unit-test cluster control plane without opt-in

Reference harness outside repo: `determinismo/pedradb-dst`.

### 11. Montanha-Store invariants (when auditing store/dcs)

Canonical map: `docs/montanha-invariants-and-tests.md`.

| ID class | Requirement |
|----------|-------------|
| **I-MAJ** | Ok put ⇒ majority applied; minority cannot Ok; no silent orphan commit |
| **I-RD** | Strong fails closed on dual-leader; LocalApplied is **not** fencing |
| **I-HA** | Leader loss → new leader serves without silent divergence |
| **I-DCS** | Create/CAS replicate; partition fails closed; heal/retry does not brick apply pipeline |

For each claim in code comments/docs, **cite the test name**. Missing test = HIGH gap (not a pass).

---

## MEDIUM severity

### 12. Bounds / OOM

- Unbounded `range` materializing full DB → prefer `range_limited` / streaming (`StreamingVisibleIter`)
- SST open loading unbounded vectors for huge tables (lazy block load gap — track against RFC-0014)
- Unbounded queues in raft/net without backpressure

### 13. API / error completeness

- Fallible ops return `Result` with semantic `CoreError` (or layer error), not panic on bad input
- Corruption errors distinguishable from not-found
- Checkpoint / verify / stats remain fail-closed on CRC

### 14. Supply chain / CI

Workspace (not metal-specific):

- [ ] `cargo deny` / `cargo audit` in CI or documented gap
- [ ] `forbid(unsafe_code)` inventory still true on pure crates
- [ ] No surprise C++/Rocks link into core

---

## Manual deep-dives (after script)

### A. Commit / fsync ordering (core)

Read: `db.rs` put/commit, `wal/writer.rs`, `tx.rs`.

Produce a step table:

| Step | I/O | Durable yet? | Visible in mem? |
|------|-----|--------------|-----------------|
| encode WriteRecord | — | no | no |
| append WAL | write | no | ? |
| sync WAL | fsync | yes if sync | ? |
| apply memtable | — | yes | yes |
| return Ok | — | must match sync policy | yes |

Crash between each step → expected state.

### B. Flush / compact publish (core)

| Step | Expected crash result |
|------|------------------------|
| Write SST tmp | no MANIFEST ref |
| fsync SST | still unreferenced |
| rename | may exist; MANIFEST must not point yet **or** recover handles |
| MANIFEST + CURRENT | consistent set |
| remove old | GC only after new version durable |

### C. Majority propose (store)

| Step | Client sees | On-disk minority | On-disk majority |
|------|-------------|------------------|------------------|
| local append only | not Ok | maybe | no |
| majority applied | Ok | yes | yes |
| NotCommitted | error / retry | ? | ? |

Heal paths must not install locks / values that majority never committed.

### D. Lock investigation (any crate with shared state)

Reuse metal-style report per lock:

```markdown
### Lock: `name` (`file:line`)
**Type:** …
**Protects / invariants:** …
**Hold duration classes:** Instant | Brief | Extended | I/O-bound
**Issues:** …
**Recommendation:** …
```

---

## Audit checklist

### Critical

- [ ] Sync path fsyncs before Ok (when sync required)
- [ ] Multi-key TX single durable unit; no half-visible
- [ ] CRC/corrupt → fail-closed
- [ ] No Env-bypassing disk I/O in engine data paths
- [ ] Core has no upper-crate imports
- [ ] Core remains `forbid(unsafe_code)` (or exception documented)

### High

- [ ] Torn WAL / orphan fragments handled; tests exist
- [ ] Flush/MANIFEST crash-safe publish
- [ ] Every durability-related `let _` / log-and-continue investigated
- [ ] Concurrent locks: no TOCTOU on version/memtable publish
- [ ] DST seams still pluggable (Env/Clock/Rng/Host/Queued RPC)
- [ ] Montanha I-MAJ / I-RD / I-HA / I-DCS mapped to live tests

### Medium

- [ ] Range/scan bounds; no accidental full-DB collect on hot API
- [ ] Errors semantic; no panic on corrupt/user input in lib code
- [ ] cargo audit/deny situation known

### Gates to run (evidence)

```bash
cargo test --workspace
cargo test -p pedradb-core --lib
cargo test -p pedradb-store --lib
cargo clippy --workspace --all-targets -- -D warnings
# if sim/oracle touched:
cargo test -p pedradb-sim --lib
cargo test -p pedradb-oracle --lib
```

Do not claim "silent_wrong=0" unless a deterministic hunt/oracle run was actually executed this session.

---

## Report format

Write the report to the user (and optionally `docs/audits/YYYY-MM-DD-audit-pedradb.md` if they ask to persist).

```markdown
## PedraDB / Montanha Audit Report

**Scope:** <crates/paths>
**Date:** <date>
**Collector:** collect-audit-data.sh (OK / partial)
**Tests run:** <commands + pass/fail>

### Overall score: X/10

### Critical
| Location | Pattern | Issue | Fix |
|----------|---------|-------|-----|
| … | … | … | … |

### High
| Location | Pattern | Issue | Fix |
|----------|---------|-------|-----|

### Medium
| Location | Pattern | Issue | Fix |
|----------|---------|-------|-----|

### Durability step tables
(commit path + flush path as above)

### Lock inventory
| Lock | Location | Protects | Issues |
|------|----------|----------|--------|

### Layer / DST seams
| Check | Result |
|-------|--------|
| core ↛ upper imports | pass/fail |
| Env used on data path | pass/fail |
| Clock/Rng for logic | pass/fail |
| Queued RPC injectable | pass/fail / n/a |

### Montanha invariants ↔ tests
| Invariant | Test | Status |
|-----------|------|--------|

### Swallowed-error investigations
(per-hit summaries)

### What's good
- …

### Residual walls (honest)
- … (do not claim Rocks/Pebble/FDB field parity)
```

---

## `--fix` mode

When the user passes `--fix` (or asks to fix):

1. Finish the audit report first (what/why).
2. Fix **CRITICAL** then **HIGH** only if the fix is local and test-backed.
3. Prefer: propagate errors, restore fsync order, route I/O through Env, add/adjust unit tests.
4. Do **not** "fix" by deleting durability checks, weakening CRC, or silencing clippy with broad allows.
5. Re-run relevant `cargo test -p …` and clippy for touched crates.
6. Update `docs/open-items.md` only if a new tracked gap appears or a listed gap closes — and only if the user wants doc updates.

---

## Anti-patterns

### "sync is slow, skip fsync in tests only"
If the code path is shared, production loses durability. Use explicit `OpenOptions { sync: false }` only when the API documents the weaker contract.

### "We'll recover any WAL we can parse"
Parsing without integrity + TX boundaries = silent wrong.

### "std::fs in one helper is fine"
One bypass blinds FailingEnv for that path forever.

### "Strong read is fine if we usually have one leader"
Dual-leader must fail closed — usually is not a consistency proof.

### "Log the error and return Ok"
On put/commit/apply/majority, that is silent wrong.

### "let _ = sync()"
CRITICAL. Never discard sync results.

### "It's only lab code"
Lab lies become production defaults. Seams and contracts are the product.

### "Silent_wrong=0 last month"
Only this session's evidence counts for the report.

### "Matches RocksDB behavior"
PedraDB is clean-room. Cite **this** code's contract and tests, not folklore.

---

## Related docs (read when section needs depth)

| Doc | Use |
|-----|-----|
| `docs/dst-seams.md` | Env/Clock/Rng/Host/Queued RPC |
| `docs/montanha-invariants-and-tests.md` | I-* ↔ tests |
| `docs/robustness-vs-rocks-pebble-fdb.md` | honest maturity bar |
| `docs/open-items.md` | known gaps |
| `docs/rfc/0009-rocksdb-class-engine.md` | engine wave 1 |
| `docs/rfc/0014-rocks-pebble-redwood-maturity.md` | maturity P0/P1 |
| `docs/rfc/0015-audit-pedradb-correctness-fixes.md` | fix backlog for audit findings (P0–P2) |
| `docs/usage.md` | public durability contract |

---

## Relationship to caixote `audit-metal`

| audit-metal | audit-pedradb |
|-------------|---------------|
| Async executor non-blocking | Sync engine OK; durability ordering |
| Lock-across-await | Lock-across-disk / TOCTOU on versions |
| QEMU/kernel CVE supply chain | cargo audit/deny + forbid(unsafe) |
| D-state host teardown | Torn WAL / MANIFEST publish / majority Ok |
| Capabilities abstraction | Env/Host/Clock/Rng + crate layer graph |

Reuse metal's **discipline** (collect once, investigate every swallow, report templates). Do not paste metal's async/CVE checklists into PedraDB findings.
