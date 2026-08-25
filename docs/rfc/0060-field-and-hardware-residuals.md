# RFC-0060: Residuais de campo e hardware — scrub at-rest, bit-flip no World, contrato de cobertura

**Status:** done (P0–P2; P2.3–P2.27 unrecognized files named FAIL)
**Updated:** 2026-08-25

## Background

- O mapa de cobertura formal ([coverage-map](../formal/coverage-map.md))
  fechou 2026-08-24 com todos os residuais tendo dono — exceto
  **field/hardware**: bit-rot em dados frios que ninguém lê, CPU errando,
  disco que perde bytes fora do caminho de leitura.
- Hoje a resposta a esses é só o **fail-closed na leitura**: WAL, MANIFEST,
  vlog e blocos SST carregam CRC32C (`wal/crc.rs`, `manifest.rs`, `vlog.rs`,
  `sst/table.rs`; `corrupt.rs` fail-closed) — corrupção é detectada **quando**
  o dado é lido, nunca servida como miss (`fail_closed` no catálogo).
- `verify-backup` valida backups; **não existe** um scrubber do DB em
  repouso ("prove que tudo que está no disco hoje decodifica e bate o CRC").
- O World injeta falhas de env (`FailingEnv`: EIO, crash, torn writes
  modelados) mas **não** vira bits de páginas já escritas — o modelo de
  "hardware mente depois do fsync" só existe como recusa honesta.

## Problems This Solves

- **Problem:** corrupção em dados frios só aparece no primeiro read (pior:
  no recovery de um desastre, quando o dado era justamente o backup de
  confiança).
- **Problem:** "CRC em tudo" é um claim verbal — nenhuma superfície audita
  qual estrutura está coberta e qual não (ex.: um `CURRENT` de 1 linha sem
  CRC fecharia o DB?).
- **Problem:** bit-flip pós-escrita não tem modelo determinístico no World —
  os oráculos de silent-wrong nunca exercitaram esse eixo.

## Proposed Solution

- Um comando de produto que percorre **todo** o DB em repouso (SSTs, vlog,
  WAL segments, MANIFEST/CURRENT) decodificando e checando CRC — o operador
  pode provar, sob demanda ou em cron (`maintain --every`), que o estado
  atual é íntegro.
- Um modelo de falha `BitFlip` no World (determinístico, seeded, vira bits
  de páginas já duradas) com oráculo: leitura subsequente **sempre** erra
  fail-closed ou lê o valor certo — nunca erra silencioso.
- Uma tabela de cobertura de checksums auditada no coverage-map (estrutura ⇒
  checksum ⇒ onde é verificada), com buracos nomeados.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** `pedra verify <db_path>`: percorre cada SST (todos os blocos),
  vlog (todos os records), WAL segments e MANIFEST; decodifica + CRC; sai 0
  com relatório (`files=, blocks=, bytes=, errors=0`) ou falha listando o
  arquivo/offset corrompido — status: `done` (`verify_at_rest` + `pedra verify`)
- [x] **P0.2** Teste de mutante: DB fechado, flip de 1 byte em um bloco SST e
  em um record do vlog ⇒ `pedra verify` acusa exatamente aquele arquivo (e o
  read path continua fail-closed) — status: `done`
  (`verify_flags_corrupted_sst_block`, `verify_flags_corrupted_vlog_record`)

### P1 — next wave (depends on P0 or clearly deferrable)

- [x] **P1.1** `pedra maintain --verify`: inclui o scrub do P0 no ciclo de
  manutenção (cron substitute já existe) — status: `done`
- [x] **P1.2** World `BitFlip`: ação de schedule determinística que vira N
  bits de páginas duradas; oráculo `silent_wrong==0` (fail-closed ou valor
  certo); teste de mutante (flip que o modelo não aplica ⇒ oráculo não
  vacuo) — status: `done` (`bitflip_never_silent_wrong`,
  `bitflip_unapplied_mutant_leaves_verify_clean`)

### P2 — later / polish

- [x] **P2.1** Tabela de cobertura de checksums no coverage-map (estrutura ⇒
  checksum ⇒ verificado-on-read ⇒ verificado-on-scrub), buracos nomeados
  (ex.: `CURRENT`, arquivos temporários de compactação) — status: `done`
- [x] **P2.2** Statement de residual de hardware no coverage-map/LEDGER:
  o que é coberto por CRC+scrub, o que depende de hardware ECC, o que o
  TCG (RFC-0052 P2) ainda separa — status: `done`
- [x] **P2.3** Scrub `CURRENT`: parse do ponteiro de uma linha + destino
  `MANIFEST-*` existe (mesmo contrato de `manifest::load`); lixo ou
  dangling ⇒ `pedra verify` nomeia `CURRENT`. Sem CRC no ponteiro (o
  buraco de checksum fica nomeado) — status: `done`
  (`check_current_pointer`; tests `verify_flags_garbage_current`,
  `verify_flags_dangling_current`)
- [x] **P2.4** Scrub do tier de história: `history/MANIFEST` (CRC `PHST`),
  `seg-*.hist` (CRC por record), `seg-*.bloom` (CRC do body). Diretório
  `history/` deixa de ser lido como ficheiro. Sem CRC no `CURRENT` —
  status: `done` (`walk_history_dir`; tests
  `verify_flags_corrupted_history_segment`,
  `verify_flags_corrupted_history_manifest`,
  `verify_clean_db_with_history_reports_zero_errors`)
- [x] **P2.5** `xor_durable_bits` / World `BitFlip` incluem o tier `history/`
  no inventário; CLI `pedra verify` nomeia `CURRENT` lixo — status: `done`
  (`xor_durable_bits_can_flip_history_segment`,
  `pedra_verify_flags_garbage_current`)
- [x] **P2.6** Scrub `CHANGELOG` (CRC cache): `pedra verify` nomeia o ficheiro
  podre; `Db::open` continua F33 (quarentena, não brick) — status: `done`
  (`verify_flags_corrupted_changelog`, `pedra_verify_flags_corrupted_changelog`)
- [x] **P2.7** Scrub `CHECKPOINT` (CRC `PDBCKP01`/`PDBCKP02`) quando o ficheiro
  existe (destino de `create_checkpoint`); DB vivo sem o ficheiro continua
  limpo — status: `done` (`verify_flags_corrupted_checkpoint_meta`)
- [x] **P2.8** `BackupEngine::verify_backup` / `pedra verify-backup` corre
  `verify_at_rest` (não só `verify_checksums`); CHANGELOG podre no base
  falha o verify-backup — status: `done` (`verify_backup_runs_at_rest_scrub`)
- [x] **P2.9** CRC de `wal/*.warch` no `verify_backup` / `verify_wal_archive`
  — status: `done` (`verify_backup_flags_corrupt_warch`)
- [x] **P2.10** CLI `pedra backup` + `pedra verify-backup`: base limpo passa;
  CHANGELOG podre no `base-000001` falha — status: `done`
  (`pedra_verify_backup_ok`, `pedra_verify_backup_flags_poison_changelog`)
- [x] **P2.11** `RemoteTier::verify` / `pedra archive verify`: CRC de cada
  `seg-*.hist` (e sidecar bloom) listado no manifesto remoto — status: `done`
  (`remote_verify_flags_corrupt_segment`,
  `pedra_archive_verify_empty_is_clean`,
  `pedra_archive_verify_ok_then_flags_poison`)
- [x] **P2.12** `LATEST` CRC: o pointer `MANIFEST-n\\ncrc32c` só é fiável se o
  CRC bater; senão fallback para a geração intacta mais nova. `archive
  verify` nomeia o mismatch — status: `done`
  (`remote_manifest_generations_latest_and_walkback`,
  `remote_verify_flags_corrupt_latest`,
  `pedra_archive_verify_flags_corrupt_latest`)
- [x] **P2.13** `verify_at_rest` desce a diretórios aninhados que são um DB
  (`CURRENT` ou `CHECKPOINT` — `base-*` de backup) e prefixa os FAIL —
  status: `done` (`verify_walks_nested_base_checkpoint`)
- [x] **P2.14** `pedra verify` numa raiz de backup (`CATALOG`) também CRC-walk
  `wal/*.warch` — status: `done` (`pedra_verify_backup_root_flags_corrupt_warch`)
- [x] **P2.15** `CURRENT` ganha linha CRC32C hex do MANIFEST (formato `LATEST`);
  uma linha legado ainda abre; mismatch é fail-closed — status: `done`
  (`current_pointer_legacy_and_crc_mismatch`, `verify_flags_current_crc_mismatch`,
  `verify_checksums_fails_on_current_crc_mismatch`,
  `pedra_verify_flags_current_crc_mismatch`)
- [x] **P2.16** Leftover `*.tmp` (CURRENT.tmp / MANIFEST-*.tmp / *.sst.tmp)
  named FAIL — not inventory, open GCs them; scrub no longer skips silently
  — status: `done` (`verify_flags_leftover_install_temp`)
- [x] **P2.17** `CATALOG` (`PDBCAT01`) magic+CRC in `verify_at_rest`; poison
  after `BackupEngine::open` still fails `verify_backup` / `pedra verify` —
  status: `done` (`verify_flags_corrupted_catalog`,
  `verify_backup_flags_corrupt_catalog`,
  `pedra_verify_backup_root_flags_corrupt_catalog`)
- [x] **P2.18** `wal/*.warch` (`PDBWAR01`) walked by `verify_at_rest` (library,
  not only CLI `BackupEngine::verify_wal_archive`) — status: `done`
  (`verify_flags_corrupted_warch`)
- [x] **P2.19** `create_checkpoint` copies the two-line CURRENT CRC pointer;
  dest `verify_at_rest` is clean; dest CRC flip names CURRENT — status: `done`
  (`verify_checkpoint_copies_current_crc`)
- [x] **P2.20** World `BitFlip` inventory includes `CURRENT`, `MANIFEST-*`,
  `CHANGELOG`, `CHECKPOINT` (not only SST/WAL/vlog/history) — status: `done`
  (`xor_durable_bits_can_flip_current`)
- [x] **P2.21** CLI `pedra verify` names leftover `CURRENT.tmp` — status: `done`
  (`pedra_verify_flags_leftover_install_temp`)
- [x] **P2.22** Scrub `CORRUPTLOG` (TSV `ts\tkind\toffset`; no CRC): garbage
  is a named FAIL, valid journal is clean — status: `done`
  (`verify_flags_garbage_corruptlog`, `pedra_verify_flags_garbage_corruptlog`)
- [x] **P2.23** `pedra inspect` reports `current_crc=absent|legacy|ok|mismatch`
  — status: `done` (`inspect_classifies_current_crc`,
  `pedra_inspect_reports_current_crc_ok`)
- [x] **P2.24** Scrub `CHANGELOG.corrupt` (F33 quarantine) with the same CRC
  as `CHANGELOG`; leftover poison is a named FAIL, open still Ok —
  status: `done` (`verify_flags_corrupted_changelog_quarantine`,
  `pedra_verify_flags_changelog_quarantine`)
- [x] **P2.25** Leftover `VALUES.vlog.adopt` named FAIL (open GCs it) —
  status: `done` (`verify_flags_leftover_vlog_adopt`,
  `pedra_verify_flags_leftover_vlog_adopt`)
- [x] **P2.26** Compat `CFREG` CRC line (`c:` hex, CURRENT-shaped); legacy
  without CRC still opens; mismatch fail-closed on open and `pedra verify`
  — status: `done` (`cfreg_crc_mismatch_fails_closed`,
  `cfreg_legacy_without_crc_still_opens`,
  `verify_flags_corrupted_cfreg`, `pedra_verify_flags_corrupted_cfreg`)
- [x] **P2.27** Unrecognized leftover files (`junk.dat`, stray `wal/*`) are
  named FAIL; `LOCK` and dotfiles (`.DS_Store`) stay skip — status: `done`
  (`verify_flags_unrecognized_file`, `pedra_verify_flags_unrecognized_file`)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | `pedra verify` — scrub at-rest | done | `verify_at_rest` + CLI `verify` | 2026-08-24 |
| P0.2 | p0 | Mutante do scrubber (SST + vlog) | done | `verify_flags_corrupted_*` | 2026-08-24 |
| P1.1 | p1 | `maintain --verify` | done | `pedra maintain --verify` | 2026-08-24 |
| P1.2 | p1 | World `BitFlip` + oráculo silent-wrong | done | `Action::BitFlip` + 2 testes | 2026-08-24 |
| P2.1 | p2 | Tabela de cobertura de checksums | done | coverage-map | 2026-08-24 |
| P2.2 | p2 | Statement de residual de hardware | done | coverage-map + LEDGER L50 | 2026-08-24 |
| P2.3 | p2 | Scrub parse+exists de `CURRENT` | done | `check_current_pointer` + 2 testes | 2026-08-24 |
| P2.4 | p2 | Scrub CRC do tier `history/` | done | `walk_history_dir` + 3 testes | 2026-08-24 |
| P2.5 | p2 | BitFlip+CLI cobrem history/CURRENT | done | xor hist + `pedra_verify_flags_garbage_current` | 2026-08-24 |
| P2.6 | p2 | Scrub CRC do CHANGELOG (F33 intacto) | done | `verify_flags_corrupted_changelog` + CLI | 2026-08-24 |
| P2.7 | p2 | Scrub CRC do `CHECKPOINT` | done | `verify_flags_corrupted_checkpoint_meta` | 2026-08-24 |
| P2.8 | p2 | `verify-backup` usa `verify_at_rest` | done | `verify_backup_runs_at_rest_scrub` | 2026-08-24 |
| P2.9 | p2 | CRC dos incrementos `wal/*.warch` | done | `verify_wal_archive` + `verify_backup_flags_corrupt_warch` | 2026-08-24 |
| P2.10 | p2 | CLI `verify-backup` produto | done | `pedra_verify_backup_ok` + poison CHANGELOG | 2026-08-24 |
| P2.11 | p2 | `pedra archive verify` CRC remoto | done | `RemoteTier::verify` + `remote_verify_flags_corrupt_segment` | 2026-08-24 |
| P2.12 | p2 | CRC do pointer `LATEST` | done | parse+crc; fallback; `remote_verify_flags_corrupt_latest` | 2026-08-25 |
| P2.13 | p2 | Scrub aninhado `base-*` / checkpoint | done | `verify_walks_nested_base_checkpoint` | 2026-08-25 |
| P2.14 | p2 | `pedra verify` + warch na raiz de backup | done | `pedra_verify_backup_root_flags_corrupt_warch` | 2026-08-25 |
| P2.15 | p2 | CRC trailer opcional no `CURRENT` | done | `parse_current_pointer` + 2 testes | 2026-08-25 |
| P2.16 | p2 | leftover `*.tmp` named FAIL | done | `verify_flags_leftover_install_temp` | 2026-08-25 |
| P2.17 | p2 | CATALOG CRC on-scrub | done | `verify_flags_corrupted_catalog` + CLI | 2026-08-25 |
| P2.18 | p2 | `wal/*.warch` on-scrub (library) | done | `verify_flags_corrupted_warch` | 2026-08-25 |
| P2.19 | p2 | checkpoint copies CURRENT CRC | done | `verify_checkpoint_copies_current_crc` | 2026-08-25 |
| P2.20 | p2 | BitFlip inventory CURRENT/MANIFEST | done | `xor_durable_bits_can_flip_current` | 2026-08-25 |
| P2.21 | p2 | CLI leftover `*.tmp` | done | `pedra_verify_flags_leftover_install_temp` | 2026-08-25 |
| P2.22 | p2 | CORRUPTLOG parse-walk | done | `verify_flags_garbage_corruptlog` + CLI | 2026-08-25 |
| P2.23 | p2 | inspect CURRENT CRC class | done | `inspect_classifies_current_crc` + CLI | 2026-08-25 |
| P2.24 | p2 | CHANGELOG.corrupt CRC | done | `verify_flags_corrupted_changelog_quarantine` + CLI | 2026-08-25 |
| P2.25 | p2 | leftover vlog adopt FAIL | done | `verify_flags_leftover_vlog_adopt` + CLI | 2026-08-25 |
| P2.26 | p2 | CFREG CRC + scrub | done | `verify_flags_corrupted_cfreg` + open fail-closed | 2026-08-25 |
| P2.27 | p2 | unrecognized leftover files FAIL | done | `verify_flags_unrecognized_file` + CLI | 2026-08-25 |

## Acceptance Criteria

- **Tests** (unit + e2e scenarios named)
  - P0: `verify_flags_corrupted_sst_block`, `verify_flags_corrupted_vlog_record`
    (mutantes de 1 byte em DB fechado); `verify_clean_db_reports_zero_errors`.
  - P1: teste World `bitflip_never_silent_wrong` + mutante de não-vacuidade.
  - P2.3: `verify_flags_garbage_current`, `verify_flags_dangling_current`.
  - P2.4: `verify_flags_corrupted_history_segment`,
    `verify_flags_corrupted_history_manifest`,
    `verify_clean_db_with_history_reports_zero_errors`.
  - P2.5: `xor_durable_bits_can_flip_history_segment`,
    `pedra_verify_flags_garbage_current`.
  - P2.6: `verify_flags_corrupted_changelog`,
    `pedra_verify_flags_corrupted_changelog` (open still Ok — F33).
  - P2.7: `verify_flags_corrupted_checkpoint_meta`.
  - P2.8: `verify_backup_runs_at_rest_scrub`.
  - P2.9: `verify_backup_flags_corrupt_warch`.
  - P2.10: `pedra_verify_backup_ok`, `pedra_verify_backup_flags_poison_changelog`.
  - P2.11: `remote_verify_flags_corrupt_segment`,
    `pedra_archive_verify_empty_is_clean`,
    `pedra_archive_verify_ok_then_flags_poison`.
  - P2.12: `remote_verify_flags_corrupt_latest` (LATEST CRC; fallback
    still serves newest intact gen);
    `pedra_archive_verify_flags_corrupt_latest`.
  - P2.13: `verify_walks_nested_base_checkpoint`.
  - P2.14: `pedra_verify_backup_root_flags_corrupt_warch`.
  - P2.15: `current_pointer_legacy_and_crc_mismatch`,
    `verify_flags_current_crc_mismatch`,
    `verify_checksums_fails_on_current_crc_mismatch`,
    `pedra_verify_flags_current_crc_mismatch`.
  - P2.16: `verify_flags_leftover_install_temp`.
  - P2.17: `verify_flags_corrupted_catalog`,
    `verify_backup_flags_corrupt_catalog`,
    `pedra_verify_backup_root_flags_corrupt_catalog`.
  - P2.18: `verify_flags_corrupted_warch`.
  - P2.19: `verify_checkpoint_copies_current_crc`.
  - P2.20: `xor_durable_bits_can_flip_current`.
  - P2.21: `pedra_verify_flags_leftover_install_temp`.
  - P2.22: `verify_flags_garbage_corruptlog`,
    `pedra_verify_flags_garbage_corruptlog`.
  - P2.23: `inspect_classifies_current_crc`,
    `pedra_inspect_reports_current_crc_ok`.
  - P2.24: `verify_flags_corrupted_changelog_quarantine`,
    `pedra_verify_flags_changelog_quarantine`.
  - P2.25: `verify_flags_leftover_vlog_adopt`,
    `pedra_verify_flags_leftover_vlog_adopt`.
  - P2.26: `verify_flags_corrupted_cfreg`,
    `cfreg_crc_mismatch_fails_closed`,
    `cfreg_legacy_without_crc_still_opens`,
    `pedra_verify_flags_corrupted_cfreg`.
  - P2.27: `verify_flags_unrecognized_file`,
    `pedra_verify_flags_unrecognized_file`.
- **Telemetry / Analytics**
  - none — o relatório do `pedra verify` é stdout/exit-code (produto de
    operação, não métrica nova).
- **Documentation**
  - Coverage-map ganha a linha "field/hardware" apontando para este RFC
    (já escrito como draft na criação); open-items atualizado.
- **Screenshots**
  - backend-only — não se aplica.

## Out of scope (non-goals)

- Provar o hardware (ECC, midia) — o contrato é detecção fail-closed +
  scrub; o residual que sobra vive no statement do P2.2.
- Paridade de throughput do scrubber (é ferramenta de operação, não bench).
- Caixa TCG (RFC-0052 P2) — este RFC cobre o eixo campo/hardware **no
  processo e no modelo**, não a execução sob hypervisor verificado.
- Redundância/replicação de arquivo (Montanha cobre replicação de nó; aqui
  é integridade single-file).
