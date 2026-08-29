# RFC: 0062 — `rocksdb-compat` como substituto estrito: só vantagem, nunca defeito

**Status:** in-progress
**Updated:** 2026-08-27
**Parents:** [0054](0054-close-official-gaps.md),
[0059-substitution](0059-substitution-anti-overindex-linux-gate.md),
[0047](0047-compat-dropin-failure-profile.md),
[0055](0055-rocks-write-pipeline.md)
**Evidence:** [`docs/reports/2026-08-25-compat-strict-substitute.md`](../reports/2026-08-25-compat-strict-substitute.md)

**Barra deste RFC (dono, 2026-08-25; OOTB 2026-08-27):** o crate `rocksdb-compat` é um **substituto** de rust-rocksdb. Config **out of the box** = factory **C++ Rocks** (`sync=false`, 64 MiB memtable; Darwin `set_sync(true)` = `F_FULLFSYNC` como o CMake). Não o `librocksdb-sys` 0.16 sem `HAVE_FULLFSYNC`. Paridade oficial = `sync=false`. 100% observável. Só vantagem. Nunca defeito. Âmbito = o crate.

**O que “100%” significa aqui:** programa P contra rust-rocksdb 0.21/0.22, reconstruído contra o compat/shim: (S1) compila a superfície que P chama, (S2) mesmo KV, (S3) nunca silent-wrong, (S4) throughput ≥ Rocks com os **mesmos** `WriteOptions.sync` que P setou, Linux, **min de 3 rounds > 1.0**, (S5) knobs ou fazem o nome ou não deixam P mais lento. Não é ABI C++ nem abrir diretório SST Rocks.

## Background

- Default do **compat** já é Rocks-shaped: `Options::sync = false` (RFC-0054). O substituto na config que os hosts usam é a coluna A (async vs `sync=false`), **não** G1 vs async.
- Linux que existe: caixote `brasil` = **um** host, AMD Threadripper PRO 3975WX, 4 vCPU virt. Metal extra: Railway AMD EPYC 9655 48 vCPU. **Não há Intel** na pool. Intel deixa de ser slice.
- Coluna A Linux hoje: 13/16 ≥ 2×. `deps_raftlog` min < 1× (0.78 isolado pré-pwrite; **0.979** mediana pós-pwrite, p50 empatado; metal pré-pwrite 0.81). Isso é o defeito de velocidade. 2× neste shape 1c, p50 empatado, não é o alvo — **min > 1.0 sempre** é.
- A tabela 0.001× (G1 vs Rocks default, 1c) **não** é full-sync. Full-sync same-class (ambos `sync=true`) no Darwin antigo era ~0.7–1.4×, não 1000×. AGENTS.md “nunca batemos o Rocks” no sync-peer = não vender vitória contra o Rocks **lento** como cartaz. O substituto **tem** de ganhar a coluna B (ambos sync) também.
- Compile: Surreal 1.5.4 já substitui. Faltam os **nomes** `Checkpoint` / `BackupEngine`. TransactionDB/Env/Titan = v2 (host 2PL não compila — honesto, não no-op).

## Problems This Solves

- **Problem:** `deps_raftlog` no Linux ainda não é >1× em **todo** round — o substituto perde num shape oficial.
- **Problem:** G1-vs-async foi citado como “somos ruins em full sync”. Não é a coluna do crate.
- **Problem:** Intel / TLS / seL4 / kernel FailClosed não são defeitos do compat e estavam no caminho crítico.
- **Problem:** knobs no-op e tipos em falta (`Checkpoint`, `BackupEngine`) são defeitos de substituto (compile ou capacidade).
- **Problem:** `kvrocks_blob_set` parked com rounds <1× — “nunca defeito” des-estaciona.

## Proposed Solution

1. Gate único do substituto: **min_ratio > 1.0** na coluna que o host pediu (A se `sync=false`, B se `sync=true`), todas as shapes, Linux que temos (4 vCPU + metal quando couber).
2. Primeiro número: remesura Linux coluna A **com `pwrite`**, `rm` do DB entre shapes (sem 1.1M leftover de `ycsb_c_big`).
3. Se raftlog min≤1.0: um corte no p99 (encode / 16 inserts). Não skiplist, não journal-CF, não Intel.
4. Coluna B Linux (ambos `sync=true`) no mesmo gate. Não citar 0.001× como full-sync.
5. S1: `Checkpoint` + `BackupEngine` wrapping o que já existe.
6. S5: inventário Wired / Inert-provado / v2. CRC nunca desliga.
7. v2: TransactionDB, Env, Titan, CFs físicos — só host nomeado que não compila.

## Delivery slices (mandatory)

### P0 — must ship first (coluna A Linux min>1.0 — o defeito que o host sente)

- [x] **P0.1** Relatório launch-readiness (histórico) — status: `done`
- [x] **P0.2** Honesty freeze 2× G1 stale + addendum compat-only (G1 ≠ full-sync; Intel não é meta) — status: `done`
      (`docs/reports/2026-08-25-compat-strict-substitute.md`)
- [x] **P0.3** Bateria Linux coluna A, write path `pwrite`, 3 rounds, `rm` entre shapes, 4 vCPU caixote AMD; gate `min > 1.0` **todas** as oficiais incluindo raftlog e blob — status: `done`
      (primeira medida `P03_FAIL` min=0.745; fechado pelo P0.4)
- [x] **P0.4** Se P0.3 raftlog min≤1.0: um corte nomeado no p99 (encode WAL **ou** 16 inserts), A/B na mesma VM, até min>1.0 — status: `done`
      (`LAST_CF` write-through; Linux 4 vCPU `RESULT=P04_PASS` min_ratio=**1.014** 17/17. [`findings/2026-08-25-linux-p04`](../../findings/2026-08-25-linux-p04/README.md))

### P1 — next wave (full-sync same-class + compile S1 + knobs S5)

- [x] **P1.1** Coluna B quieta 3/3, min>1.0: Darwin smoke 2026-08-25 (load 20) já existe — ycsb_a p50 empatado 4.85 vs 4.73 ms; Linux 4 vCPU — status: `done`
      (`lone_sync_commit`; Linux `RESULT=P11_PASS` min_ratio=**1.013** 17/17, raftlog 1.013/1.019/1.021. [`findings/2026-08-25-linux-p11`](../../findings/2026-08-25-linux-p11/README.md))
- [x] **P1.2** `Checkpoint` + `BackupEngine` com os nomes rust-rocksdb, wrap `create_checkpoint` / `pedradb-ops`; smoke host 0.22 — status: `done`
      (`checkpoint::Checkpoint`, `backup::BackupEngine` + stub `Env` só para `BackupEngine::open`; `checkpoint_and_backup_engine_roundtrip`; alias-smoke)
- [x] **P1.3** Inventário S5 máquina-checkable (Wired / Inert-provado / v2); `set_verify_checksums(false)` **não** desliga CRC — status: `done`
      (`KNOB_INVENTORY`; `set_verify_checksums(false)` / `set_paranoid_checks(false)` / skip-any / `NoChecksum` → `ErrorKind::NotSupported`; CRC stays on)
- [x] **P1.4** Metal AMD (EPYC ou o 4 vCPU se P0.3 já for o único Linux) coluna A min>1.0 — prova “todos os Linux que medimos”, não Intel — status: `done`
      (único Linux na pool: caixote 4 vCPU Threadripper PRO 3975WX. `RESULT=P04_PASS` min **1.014** 17/17. EPYC extra não é meta.)

### P2 — later (v2 superfície; não bloqueia v1 Surreal 1.5)

- [x] **P2.1** `TransactionDB` 2PL — status: `done`
      (exclusive key locks; `Busy`/`TimedOut`; `get_for_update`; 1PC — `prepared_transactions` vazio)
- [x] **P2.2** `Env` / `SstFileManager` superfície 0.22 — status: `done`
      (`Env` thread-pool setters stored; `Options::set_env`; `SstFileManager` + `set_sst_file_manager`. Pools/rate Inert: Pedra compact worker is one thread; caps stored, compact does not stall yet.)
- [x] **P2.3** CFs físicos (N LSM, 1 WAL) — status: `done` → [0065](0065-physical-column-families-one-wal.md) P0+P1 (SST/mem/stall por CF, 1 WAL). P2.1 raftdb path remaining. Titan/UDT **fora**.
- [ ] **P2.4** `crates.io` `rocksdb-compat` (nome **não** `rocksdb`) depois de P0+P1 — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | relatório quatro respostas | done | docs/reports/2026-08-25-launch-readiness.md | 2026-08-25 |
| P0.2 | p0 | addendum compat-only + freeze stale | done | compat-strict-substitute.md | 2026-08-25 |
| P0.3 | p0 | Linux coluna A pwrite min>1.0 todas | done | P03_FAIL 0.745; fechado no P0.4 | 2026-08-25 |
| P0.4 | p0 | p99 raftlog se P0.3 falhar | done | LAST_CF; P04_PASS min 1.014 17/17 | 2026-08-25 |
| P1.1 | p1 | Linux coluna B ambos sync min>1.0 | done | p11j P11_PASS min 1.013 17/17 | 2026-08-27 |
| P1.2 | p1 | Checkpoint + BackupEngine nomes | done | checkpoint.rs + backup.rs + alias-smoke | 2026-08-25 |
| P1.3 | p1 | knobs S5 | done | knobs.rs + g2_setters_are_not_supported | 2026-08-25 |
| P1.4 | p1 | metal AMD coluna A min>1.0 | done | 4 vCPU = único Linux; P04_PASS 1.014 | 2026-08-25 |
| P2.1 | p2 | TransactionDB 2PL | done | txn.rs LockTable exclusive | 2026-08-27 |
| P2.2 | p2 | Env + SstFileManager 0.22 names | done | env.rs; set_env; set_sst_file_manager | 2026-08-27 |
| P2.3 | p2 | CF físico (N LSM 1 WAL) | done | RFC-0065 P0+P1 | 2026-08-27 |
| P2.4 | p2 | crates.io rocksdb-compat | todo | after P0+P1 | 2026-08-25 |

## Acceptance Criteria

- **Tests**
  - P0.3: `parity_gate_closed.py` com floor **1.0** em **todas** as oficiais (raftlog e blob inclusos); JSON `sync: false`, `peer_policy: rocks-default`; `min_ratio > 1.0`.
  - P0.4: A/B na mesma VM; o corte que fechar o min é o que fica; os outros revertem.
  - P1.1: mesmo gate, `ROCKS_PARITY_SYNC=1` **e** compat `set_sync(true)` (coluna B). Compare **não** usa isto como cartaz vs default.
  - P1.2: crate smoke `use rocksdb::{Checkpoint, backup::BackupEngine}`; backup+restore+verify de 1 key.
  - P1.3: tabela gerada; teste que `set_verify_checksums(false)` deixa CRC ligado (mutante de 1 byte ainda fail-closed).
  - P2.1: `TransactionDB::open_default` + `transaction` put/commit; second writer `lock_timeout=0` → `TimedOut`; rollback releases lock; `get_for_update` exclusive.
  - P2.2: `Env::new` + `set_background_threads`; `SstFileManager::new` + `Options::set_env` / `set_sst_file_manager`; alias-smoke.
- **Telemetry / Analytics**
  - Nenhuma tabela do substituto lidera com coluna C (G1 vs async). Coluna C, se publicada, leva a frase “1c paga a barreira, eles não — não é full-sync”.
- **Documentation**
  - addendum §0–§5; este RFC; `docs/rocksdb-compat.md` ganha S1–S5 quando P1.3 aterrissar.
- **Screenshots**
  - backend-only.

## Out of scope

- Intel como meta (não há na pool; não bloqueia S4).
- 2× em `deps_raftlog` 1c (p50 empatado).
- Coluna C como gate do substituto (G1 vs Rocks async 1c é física, não defeito do crate).
- Montanha TLS, encrypt-at-rest, seL4, RFC-0038 kernel FailClosed (compat já é PIT).
- journal-CF, skip-fsync, Titan no v1, skip-any, `paranoid_checks=false`.
- Publicar o nome `rocksdb` no crates.io.
- Abrir diretório SST C++ Rocks.
- Relitigar o peer oficial da coluna A (`sync=false`).
