# RFC: 0149 — coluna A, maioria das shapes >3× vs Rocks default

**Status:** in-progress (P0 done)
**Updated:** 2026-08-29
**Parents:** [0065](0065-physical-column-families-one-wal.md) (CFs físicas),
[0062](0062-launch-readiness-remaining-gaps.md) (substituto min>1.0),
[0054](0054-close-official-gaps.md)
**Evidence:** [`findings/2026-08-29-3x-baseline/`](../../findings/2026-08-29-3x-baseline/)

## Background

- Gate oficial do substituto continua min>1.0 (Linux P04 17/17, min 1.014).
- Pedido: **maioria** das 17 shapes oficiais >3× na coluna A (async vs Rocks `sync=false`).
- Baseline desta árvore (Mac, load ~4, 2026-08-29): **5/17 >3×**, `ycsb_a` **0.931×** (era ~3.5–4.8×). Pedra 2.08M→0.63M qps no A; o peer Rocks não acelerou.
- Causa medida no código, não na teoria: `maybe_auto_flush` com `physical_cfs` chamava `MemTable::cf_families()`, que percorre **todas** as keys do tail+map. Cada put 1c ficou O(entries).

## Problems This Solves

- **Problem:** CFs físicas (0065) tornaram o put 1c linear no tamanho da memtable.
- **Problem:** `family_of_user_key` alocava `String` por op.

## Proposed Solution

1. Auto-flush: se o uso global está abaixo de todos os caps, return. Senão, checar só os nomes em `physical_cfs` via `approx_memory_usage_cf` (já mantido em `cf_bytes`).
2. Família do user key: `&str` contra a lista registada, sem `String`.
3. Remedir as 17 oficiais. Alvo: **>3× na maioria** (≥9/17). Linux continua o cartaz min>1.0.

## Delivery slices (mandatory)

### P0 — must ship first (put deixa de ser O(entries))

- [x] **P0.1** `maybe_auto_flush` O(CFs); early-out sob o cap — status: `done`
- [x] **P0.2** Remedir 17 oficiais coluna A neste host; maioria >3× — status: `done`
      ([`findings/2026-08-29-3x-p01`](../../findings/2026-08-29-3x-p01/README.md): **12/17** median >3×, 3 rounds; A 0.93→3.82)

### P1 — next wave

- [x] **P1.1** Encode `write_cf_owned` agrupa por CF + prefixo reutilizado — status: `done`
      (apply intercalava lock/default 32×. Isolado 3/3: lock 1.85→**2.09**, apply 1.91→**2.08**. Mem 9µs/commit num BTree de 267k — não fecha 3× nestas 5.)

### P2 — later

- [ ] **P2.1** Linux 4 vCPU, mesma regra maioria >3× — status: `doing`
      (VM caixote TAP nexthop. Medida no metal 4-CPU NVMe: **7/17 FAIL**,
      [`findings/2026-08-29-linux-p149/`](../../findings/2026-08-29-linux-p149/).
      Não é QEMU virt. Mac P0 12/17 continua a única maioria.)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | auto-flush O(CFs) | done | db.rs maybe_auto_flush | 2026-08-29 |
| P0.2 | p0 | 17 shapes maioria >3× | done | 12/17 med >3× 3x-p01 | 2026-08-29 |
| P1.1 | p1 | group-by-CF encode | done | write_cf_owned; lock/apply ~2.1× isolado | 2026-08-29 |
| P2.1 | p2 | Linux maioria >3× | doing | TAP blocked; NVMe 4CPU 7/17 FAIL | 2026-08-29 |

## Acceptance Criteria

- **Tests:** `maybe_auto_flush_physical_cf_is_not_linear_in_keys`; `auto_flush_default_does_not_flush_lock` continua verde.
- **Telemetry / Analytics:** compare JSON em `findings/2026-08-29-3x-*`; coluna A only.
- **Documentation:** este RFC; open-items uma linha.
- **Screenshots:** backend-only.

## Out of scope

- G1 vs Rocks async. 2× em raftlog 1c. Titan. Publicar crates.io (0062 P2.4).
