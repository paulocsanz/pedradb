# RFC: 0090 — Raft hard-state CRC mismatch is not Ok (never elect on a flipped term)

**Status:** done
**Updated:** 2026-08-27
**Parents:** [0076](0076-crc-mismatch-not-ok.md), [0089](0089-history-latest-crc-mismatch-not-ok.md), [0015](0015-montanha-tcp-fail-injection.md), [0061](0061-residuals-sel4-ironfleet.md)

**Residual:** `R-hardware` (not `never_floor`). Axis vs FDB Sim2: **G3 disk / G8 recover** — Sim2’s disk is a model. Pedra’s `raft-meta/RAFT_HARD` (`PRFT` v2 + term/vote + trailing CRC32C) is the durable term/`voted_for`. WAL/SST/vlog/MANIFEST/history already call `crc_match_ok`. `load_hard_on` still compared `expect != got` inline. AS-IS treats any checksum as matching (flipped term served on recover). This slice names the gate on the **live Raft persist path**: mismatch is `RaftError::Persist` containing `crc mismatch`. The lie is **only the trailer** (magic/`term`/`voted_for` intact). A mid-file term flip (`hard_byte_flip_fail_stops` at byte 10) is not this tooth.

P1 gates `RAFT_COMMIT` / `RAFT_LOG` the same way. P2.1 gates store `strip_crc`, ops `CATALOG` decode, and CFREG. P2.2 records that CRC32C collision **remains** `R-crc` in `never_floor` (not a Pedra theorem).

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. CRC32C collision is still R-crc. Not an ECC proof. Production G1 stays POSIX (do not restore WAL onto io_uring).

## Background

- `RaftNode` persist/recover calls `store_hard` / `load_hard` (StdEnv on Darwin).
- CRC is checked **before** magic, so a trailer lie never reaches a parse error.
- RFC-0076 named the WAL gate; Raft meta stayed inline.

## Problems This Solves

- **Problem:** hard-state CRC lived in persist glue.
- **Problem:** AS-IS would load the intact term after a trailer lie.
- **Problem:** history/WAL used the named gate; Raft hard state did not.
- **Problem:** commit/log, store raft-meta, ops catalog, and CFREG still compared inline.

## Proposed Solution

- Production `load_hard_on`, `load_commit_on`, `decode_log`, store `strip_crc`, ops catalog `decode`, and CFREG `cfreg_payload` call `crc_match_ok`. No new `*_kernel.rs`. Do not extract `db.rs`. Collision axiom stays `R-crc`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on live hard-state load)
- [x] **P0.1** `load_hard_on` calls `crc_match_ok` — status: `done`
- [x] **P0.2** Regression: live `store_hard`, XOR trailer only, load is crc mismatch — status: `done` (`crc_mismatch_on_live_raft_hard_is_not_ok`)

### P1 — next wave
- [x] **P1.1** `load_commit_on` uses `crc_match_ok` — status: `done` (`crc_mismatch_on_live_raft_commit_is_not_ok`)
- [x] **P1.2** `decode_log` uses `crc_match_ok` — status: `done` (`crc_mismatch_on_live_raft_log_is_not_ok`)

### P2 — later
- [x] **P2.1** Store-layer `strip_crc` / ops catalog / CFREG use the same gate — status: `done`
- [x] **P2.2** Collision axiom remains R-crc — status: `done` (`never_floor` still lists `R-crc`; not a proof)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | load_hard_on gated | done | persist.rs load_hard_on | 2026-08-27 |
| P0.2 | p0 | trailer lie is crc mismatch | done | crc_mismatch_on_live_raft_hard_is_not_ok | 2026-08-27 |
| P1.1 | p1 | commit CRC | done | crc_mismatch_on_live_raft_commit_is_not_ok | 2026-08-27 |
| P1.2 | p1 | log CRC | done | crc_mismatch_on_live_raft_log_is_not_ok | 2026-08-27 |
| P2.1 | p2 | store/ops/CFREG CRC | done | strip_crc + Catalog::decode + cfreg_payload | 2026-08-27 |
| P2.2 | p2 | R-crc collision | done | never_floor still lists R-crc | 2026-08-27 |

## Acceptance Criteria

- **Tests**
  - `crc_match_ok(1,2)` false; AS-IS true.
  - `crc_mismatch_on_live_raft_hard_is_not_ok`: production `store_hard` (real `RAFT_HARD`). XOR only the last 4 CRC bytes (magic/term/vote intact). `load_hard` is Err containing `crc mismatch`. Dropping `crc_match_ok` (AS-IS) would return the stored term. Runs on Darwin. Does not submit io_uring SQEs.
  - `crc_mismatch_on_live_raft_commit_is_not_ok`: production `store_commit` (real `RAFT_COMMIT`). XOR trailer only. `load_commit` is crc mismatch. AS-IS would return the index.
  - `crc_mismatch_on_live_raft_log_is_not_ok`: production `store_log` (real `RAFT_LOG`). XOR trailer only. `load_log` is crc mismatch. AS-IS would return the entries. `log_byte_flip_fail_stops` (mid-file) is **not** this tooth.
  - `crc_mismatch_on_live_store_raft_meta_is_not_ok`: production `encode_hard`/`decode_hard` (`strip_crc`). XOR trailer only. Decode is crc mismatch.
  - `crc_mismatch_on_live_ops_catalog_is_not_ok`: production `BackupEngine::open_with_env` (real `CATALOG`, StdEnv). XOR trailer only. Reopen is crc mismatch.
  - `crc_mismatch_on_live_cfreg_is_not_ok`: production `store_cf_registry` (real `CFREG`). XOR stored CRC u32, rewrite `c:` 8 hex (prefix intact). `DB::open_cf` is crc mismatch. ASCII XOR of a hex digit is **not** this tooth.
  - Existing `hard_byte_flip_fail_stops` (payload byte 10) is **not** the hard-state tooth.
- **Telemetry / Analytics:** none — integrity invariant.
- **Documentation:** this RFC; `residuals.json` `R-hardware` close-text names 0090 P0–P2; `R-crc` stays `never` in `never_floor`.
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Restoring production WAL onto io_uring. Relitigating RFC-0074 CQE on dead wrappers.
- ECC / CPU proof. CRC collision (R-crc remains `never_floor`). Extracting `db.rs`. New `*_kernel.rs`.
- Ops `warch` CRC (not named in P2.1). History bloom fail-open prune.
- Rocks coluna A/B. crates.io.
