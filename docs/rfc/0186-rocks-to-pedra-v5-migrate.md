# RFC-0186: Migrar RocksDB → Pedra SST v5 (não abre SST C++; não é drop-in)

**Status:** done
**Updated:** 2026-09-08
**ID:** 0186
**Parents:** [0006](0006-sst-flush.md) (Pedra SST),
[0047](0047-compat-dropin-failure-profile.md) (API drop-in ≠ on-disk),
[0065](0065-physical-column-families-one-wal.md) (CF físicas),
[0077](0077-sst-crc-fate-fail-closed.md) (SST v5 + CRC),
[0159](0159-sorted-ingest-bulk-load.md) (bulk no *dest*, não no src)

> **Contrato:** Pedra **não** abre um diretório SST do C++. **Não é drop-in
> on-disk.** `rocksdb-compat` é drop-in de *API*. A migração é cópia lógica
> para um diretório Pedra novo, escrito pelo writer corrente (MANIFEST v5;
> SST v5 lz4 / v3 raw quando incompressível).

## Background

- Pedra SST começa com `PEDRSST\0`; writer corrente é **v5** (blocos lz4 +
  CRC por bloco, RFC-0077). MANIFEST começa com `PDBM` (v5 = CF por ficheiro,
  RFC-0065). `pedra migrate` hoje só reescreve **Pedra→Pedra**
  (`migrate_to_latest`).
- RocksDB C++ é BlockBasedTable: magic no *footer*, `IDENTITY` / `OPTIONS-*` /
  `MANIFEST` log, WAL numerado. Primeiros 8 bytes de um `.sst` Rocks **não**
  são `PEDRSST\0`.
- `Db::open` / `SstTable::decode` já recusam magic alheio (`bad SST magic`).
  `ingest_external_file` no compat só lê SST Pedra e re-aplica pelo WAL (G1) —
  não é hardlink de SST Rocks.
- `pedradb-core` é `#![forbid(unsafe_code)]` e **não liga C++**. Rocks entra
  só como oráculo (`pedradb-oracle` `live-rocksdb`) ou peer de bench
  (`rocksdb-parity-bench --features real`).
- Quem tem um diretório Rocks em disco não tem caminho de produto para Pedra
  v5: `open` falha, `migrate` falha, e a frase “drop-in” no README refere a
  API, não o layout.

## Problems This Solves

- **Problem:** um operador aponta Pedra a um dir Rocks e lê “bad SST magic”
  sem o próximo passo. Não há cópia oficial para SST v5.
- **Problem:** ensinar o kernel a parsear BlockBasedTable seria o drop-in
  on-disk que este produto recusa (C++ no TCB, silent-wrong de footer/CRC/
  merge/wide-column).
- **Problem:** `pedra migrate` (rewrite in-place) e “importar Rocks” são
  operações distintas; misturá-las no mesmo argv copia dados para o sítio
  errado.

## Proposed Solution

- Kernel **intocado como leitor C++**: nenhum `Db::open` / `SstTable::decode`
  admite SST sem `PEDRSST\0`. Mensagem nomeia o contrato e aponta o comando
  de cópia.
- Ops classifica o diretório (`Pedra` / `Rocks` / `Empty` / `Unknown`) pelos
  marcadores de *layout* (magic SST/MANIFEST, `IDENTITY`, `CURRENT.log`) —
  sem parsear um BlockBasedTable.
- `pedra migrate-from-rocks <src> <dst>`: checkpoint Rocks do src (read-only,
  todas as CFs), itera o snapshot *visível* de **cada** CF, grava um **dst
  novo** via `apply_batch` Pedra (`default` raw, outras `cf\0key`, MANIFEST
  v5). Merge / blob / wide-column / user-timestamps recusam. Src e dst
  distintos; src não é escrito.
- Sem a feature, o comando existe e recusa com “rebuild `--features
  from-rocks`” — o binário default continua sem librocksdb.

## Delivery slices (mandatory)

### P0 — must ship first (recusa explícita + cópia default-CF)

- [x] **P0.1** Este RFC — status: `done`
- [x] **P0.2** `DirKind` + `inspect` / `migrate_to_latest` / `Db::open`
      recusam um dir Rocks com a frase *não é drop-in* (não parseiam o SST
      C++). Testes sem toolchain C++ (marcadores sintéticos). — status: `done`
- [x] **P0.3** `migrate_from_rocks(src, dst)` (feature `from-rocks`):
      snapshot visível (iterator; deletes não ressuscitam). Dst vazio;
      crash a meio = dst incompleto, retry. Flush Pedra + **MANIFEST v5**
      + `verify_checksums`. P0 copiava só `default`; **P1.1 supersedes**
      (todas as CFs). — status: `done`
- [x] **P0.4** CLI `pedra migrate-from-rocks <src> <dst>` + docs
      (`usage.md`, `rocksdb-compat.md`, este RFC). Sem a feature: erro de
      rebuild, exit ≠ 0. — status: `done`

### P1 — next wave (CFs + volume)

- [x] **P1.1** CFs nomeadas → CFs Pedra físicas (RFC-0065), um WAL no dst,
      `cf\0key` (default raw). — status: `done`
- [x] **P1.2** Cópia por `apply_batch` (chunks de 256) no dst; relatório
      `keys` / `bytes` / `ssts` soma todas as CFs. — status: `done`
- [x] **P1.3** Recusar src com merge operator / blob (Titan / `*.blob` /
      `enable_blob_files`) / wide-column marker / `.u64ts` (fail-closed).
      Gap: `PutEntity` sem marker OPTIONS não é detectável (Rocks 8.10
      não serializa wide-column). — status: `done`

### P2 — later / polish

- [x] **P2.1** Checkpoint Rocks no src (read-only, todas as CFs) antes do
      iterator; guard apaga o temp em Ok e em Err. Src byte-idêntico.
      — status: `done`
- [x] **P2.2** Twin / dente `sst_magic_is_pedra` no kernel SST
      (`magic_kernel.rs`; AS-IS admite qualquer header). Decode +
      `DirKind` chamam o mesmo predicado. Catalog pair `sst_magic`.
      — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-08 |
| P0.2 | p0 | DirKind + recusa Rocks | done | dir_kind.rs | 2026-09-08 |
| P0.3 | p0 | migrate_from_rocks default CF → Pedra v5 | done | from_rocks.rs | 2026-09-08 |
| P0.4 | p0 | CLI + docs | done | pedra migrate-from-rocks | 2026-09-08 |
| P1.1 | p1 | CFs nomeadas → físicas | done | from_rocks.rs apply_batch + encode_cf_key | 2026-09-08 |
| P1.2 | p1 | batch / relatório keys/bytes/ssts | done | APPLY_CHUNK=256 | 2026-09-08 |
| P1.3 | p1 | merge/blob/wide-col/UDT recusam | done | OPTIONS+*.blob predicates; PutEntity sem marker = gap | 2026-09-08 |
| P2.1 | p2 | checkpoint src | done | Checkpoint::create_checkpoint + Drop guard | 2026-09-08 |
| P2.2 | p2 | kernel sst_magic_is_pedra | done | sst/magic_kernel.rs + catalog sst_magic | 2026-09-08 |

## Acceptance Criteria

- **Tests**
  - `sst_magic_is_pedra_on_cpp_header_is_not_ok` — header ≠ `PEDRSST\0` não
    é Pedra; AS-IS abençoaria.
  - `inspect_refuses_rocks_dir` / `migrate_to_latest_refuses_rocks_dir` —
    dir sintético (`IDENTITY` + MANIFEST sem `PDBM` + `.sst` sem magic
    Pedra) → erro *not drop-in*; nenhum `SstTable` servido.
  - `open_refuses_rocks_sst_dir` — `Db::open` no mesmo dir é `Err`.
  - `migrate_from_rocks_refuses_pedra_source` / `_nonempty_dest`.
  - P1.3 no-C++: `migrate_from_rocks_refuses_merge_operator_options` /
    `_titan_blob_file` / `_wide_column_options` / `_user_timestamps_options`.
  - Feature `from-rocks`: default-CF snapshot; named CFs `default`+`write`+
    `lock` copy (get_cf / `cf\0key`, MANIFEST 5, report keys/bytes);
    real merge-operator src recusa; checkpoint deixa src byte-idêntico e
    apaga o temp em Ok e em Err; `verify_checksums` ok.
  - P2.2: `sst_magic_is_pedra` admite só `PEDRSST\0`; AS-IS abençoa o
    header C++; `SstTable::decode` e `DirKind` chamam o mesmo predicado.
- **Telemetry / Analytics:** none — ops path, não coluna de paridade.
  Nenhum ratio vs Rocks é win desta migração.
- **Documentation:** este RFC; `docs/usage.md`; `docs/rocksdb-compat.md`
  (API drop-in ≠ layout); `docs/status.md`.
- **Screenshots:** none — backend-only.

## Out of scope

- Abrir um dir Rocks com `Db::open` / `rocksdb-compat::DB::open`.
- Hardlink / ingest de BlockBasedTable no LSM Pedra.
- Preservar sequence numbers, snapshots Rocks, merge operands crus,
  blob Titan, wide columns, user timestamps (P1.3 recusa; não traduz).
- Reescrever in-place o diretório src (`pedra migrate` continua Pedra→Pedra).
- Ligar librocksdb no `pedradb-core` ou no CLI default.
- Claim de “drop-in on-disk” ou de win vs Rocks `sync=true`.
