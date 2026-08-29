# RFC: 0075 — C ABI oversize `*_len` is LIMIT (no huge slice)

**Status:** done
**Updated:** 2026-08-28
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0023](0023-fdb-functional-tx-parity-and-compat-face.md)

**Residual:** `R-unsafe-capi` (not `never_floor`). Axis vs FDB Sim2: **G1 C client** — Sim2 does not run `libfdb_c`. Pedra’s in-process C ABI (`pedradb-capi`) is the product face. Marshalling already refused `len > max` before `copy_nonoverlapping` (F215). That predicate was inline in `copy_c_bytes` / `transaction_set`. AS-IS would copy any `len` (terabyte slice / ASan-miss). This slice names the gate on the **live C ABI** (Darwin and Linux): oversize `*_len` is `MONTAHA_FDB_LIMIT` and does not read.

**Refused claims:** this RFC does **not** claim “garantia total”, “sem bugs”, or a global score “mais confiável que FDB”. `SAFETY.md` is not a ∀ proof. A C caller that lies about a *capped* length is still UB. ASan harness remains the product gate.

## Background

- Handles are packed slot+generation, not `Box::into_raw`. Length caps: key 10MiB, value 100KiB, path 4096.
- `copy_c_bytes` already `if len > max { LIMIT }`. Set/get duplicated the check.
- FDB’s C tester is outside Sim2. Pedra must fail-closed on the same class of huge-len lie **before** `unsafe` copy.

## Problems This Solves

- **Problem:** the cap lived in two glue sites with no named decision.
- **Problem:** AS-IS would construct a huge slice and call `copy_nonoverlapping`.
- **Problem:** existing oversize test used a **null** handle (cap before lookup). P0 also drives a live create+tx.

## Proposed Solution

- Pure `c_len_admitted(len, max)` = `len <= max`. AS-IS always true.
- Production `copy_c_bytes` and `montanha_fdb_transaction_set` / `_get` call it. No new `*_kernel.rs`.

## Delivery slices (mandatory)

### P0 — must ship first (gate on the live C ABI)
- [x] **P0.1** `c_len_admitted` + AS-IS — status: `done`
- [x] **P0.2** `copy_c_bytes` + set/get use the gate — status: `done`
- [x] **P0.3** Regression: live db+tx, oversize key_len is LIMIT; AS-IS would admit — status: `done` (`c_len_oversize_on_live_tx_is_limit`)

### P1 — next wave
- [x] **P1.1** Path NUL walk uses a named cap (`MAX_PATH_BYTES`) kernel — status: `done` (`c_path_walk_bytes`; `c_path_walk_uses_named_cap`)
- [x] **P1.2** `capi-asan.sh` asserts the same LIMIT on the PASS binary — status: `done` (`LIMIT key/value/get/live-key` banners)

### P2 — later
- [x] **P2.1** Catalog pair + Verus twin — status: `done` (`verus/c_len.rs` + catalog `c_len`)
- [x] **P2.2** Buffer `free` table still TCB — status: `done` (`c_free_table_admitted`; no `verus/c_free.rs`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | c_len_admitted + AS-IS | done | handles.rs | 2026-08-27 |
| P0.2 | p0 | copy_c_bytes + set/get gated | done | lib.rs | 2026-08-27 |
| P0.3 | p0 | live tx oversize LIMIT | done | c_len_oversize_on_live_tx_is_limit | 2026-08-27 |
| P1.1 | p1 | path cap kernel | done | c_path_walk_uses_named_cap | 2026-08-28 |
| P1.2 | p1 | capi-asan LIMIT | done | capi-asan.sh LIMIT banners | 2026-08-28 |
| P2.1 | p2 | catalog + Verus | done | c_len.rs + catalog c_len | 2026-08-28 |
| P2.2 | p2 | free table TCB | done | c_free_table_admitted (no c_free.rs) | 2026-08-28 |

## Acceptance Criteria

- **Tests**
  - `c_len_admitted(max+1, max)` false; AS-IS true.
  - `c_len_oversize_on_live_tx_is_limit`: `montanha_fdb_database_create` + `transaction_create` (real cluster), then `transaction_set` with `key_len = MAX_C_KEY_BYTES+1` on a 1-byte buffer returns `MONTAHA_FDB_LIMIT` (does not read past the buffer). Runs on Darwin.
  - P1.1 `c_path_walk_uses_named_cap`: `c_path_walk_bytes() == MAX_PATH_BYTES` (4096); AS-IS `usize::MAX`; `c_path_nul_off_admitted(MAX_PATH_BYTES)` false; live `database_create` on a 4096-byte no-NUL buffer is NULL. `path_from_c` calls `memchr` with the named bound.
  - P1.2 `capi-asan.sh`: PASS log contains `capi_asan: LIMIT key`, `LIMIT value`, `LIMIT get`, `LIMIT live-key` (live create+tx oversize). Missing banner fails the script. ASan still required for the malicious binary.
  - P2.1 catalog pair `c_len` entry `c_len_admitted` with Verus twin (freeze of twin files; `verus` not on PATH). No new `*_kernel.rs`.
  - P2.2 `c_free_table_is_tcb`: `c_free_table_admitted()` false; AS-IS true; `verus/c_len.rs` present; `verus/c_free.rs` absent; live get+free hits the table; unknown/double-free is a no-op.
- **Telemetry / Analytics:** none — ABI safety invariant.
- **Documentation:** this RFC; `residuals.json` `R-unsafe-capi` close-text + owner 0075 (still names SAFETY.md ≠ forall).
- **Screenshots:** backend-only.

## Out of scope

- “Garantia total”, “sem bugs”, ranking “mais confiável que FDB”.
- Making C memory-safe. Restoring production WAL onto io_uring.
- Extracting `db.rs`. `never_floor`. New `*_kernel.rs`. Rocks coluna A/B. crates.io.
