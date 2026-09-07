# Mapa de cobertura formal — inventário final kernel×glue (RFC-0057 P2.3)

**Data:** 2026-08-24
**Fonte:** `scripts/formal/catalog.json` (46 pares) + freeze verde
(`python3 scripts/formal/pedra_formal.py --ci`: 223 ok / 0 fail, incl. o par novo
`group_commit`/`group_fence` desta data).

Resposta curta a "estamos o mais coberto possível?": **dentro do TCB declarado,
sim** — toda decisão com estado do caminho crítico tem kernel provado, twin
Verus e caller-lint no CI; o extrato Aeneas→Lean cobre o segundo lado da
moeda (o mesmo código de produção, como máquina concreta). O que não é
kernels **tem dono nomeado** (tabela final); o único residual sem dono até
hoje (field/hardware) ganhou [RFC-0060](../rfc/0060-field-and-hardware-residuals.md)
(draft) nesta mesma mudança.

## Números (regra de contagem explícita)

- **Kernels:** 36 arquivos `*_kernel.rs` sob `crates/*/src` (globs congelados
  do freeze + kernels modelo in-file): **6.285 LOC**. O kernel maior é
  `wal/recover_kernel.rs` (508); o novo `group_commit_kernel.rs` (2026-08-24)
  fecha a última decisão com estado do commit path.
- **Superfície formalizada:** 11 crates com pares no catálogo —
  **75.662 LOC** de `src` (inclui módulos `#[cfg(test)]` inline):
  core 39.388 · store 21.716 · raft 4.398 · http 3.053 · dcs 1.861 ·
  fdb-recipes 1.625 · fold 1.430 · apply 550 · stream 440 · replicate 854 ·
  journal 347.
- **Glue restante ≈ 69.377 LOC**: TCB **declarado, não escondido** — o freeze
  do TCB (RFC-0056 P2.5) recusa crescimento silencioso dos globs
  `crates/*/src/**/*_kernel.rs`; o lint de callers exige que cada caller chame
  o kernel da entrada; e o comportamento do glue é exercitado pelos oráculos
  DST/World (`silent_wrong=0`, reopen exato, fence fail-closed).
- Contagem anterior (relatório "100% relativo ao TCB", RFC-0056): 7.723 LOC
  kernel / 39.793 LOC handler-num subconjunto menor — regra diferente
  (ficheiros handler, não src inteiro); os dois números convivem porque o
  denominador é declarado em cada um.

## Tabela kernel × glue (os 46 pares do catálogo)

Twin `close` = tradução estrita do kernel de produção; `atom` = função
executável verificada diretamente; `model` = teorema sobre um modelo do
comportamento. "Callers" é o que o lint do freeze exige chamar a entrada.

| id | kernel | twin | callers (glue lintado) |
|----|--------|------|------------------------|
| vote | `pedradb-raft/src/vote_kernel.rs` | close | `pedradb-raft/lib.rs` |
| ae_entry | `pedradb-raft/src/ae_kernel.rs` | close | `pedradb-raft/lib.rs` |
| ae_ack | `pedradb-raft/src/ae_kernel.rs` | close | `pedradb-raft/lib.rs` |
| commit_raft | `pedradb-raft/src/commit_kernel.rs` | close, single_artifact | `pedradb-raft/lib.rs`, `pedradb-raft/net.rs` |
| lease | `pedradb-dcs/src/lease_kernel.rs` | close, single_artifact | `pedradb-dcs/command.rs` |
| dcs_apply | `pedradb-raft/src/apply_kernel.rs` | close | `pedradb-raft/lib.rs` |
| txn | `pedradb-store/src/txn_kernel.rs` | close | `pedradb-store/lib.rs` |
| compact | `pedradb-store/src/compact_kernel.rs` | close | `pedradb-store/lib.rs` |
| snapshot | `pedradb-store/src/snapshot_kernel.rs` | close | `pedradb-store/lib.rs` |
| si_reader | `pedradb-store/src/si_kernel.rs` | close | `pedradb-store/lib.rs` |
| si_read | `pedradb-store/src/si_kernel.rs` | close | `pedradb-store/lib.rs` |
| index_val | `pedradb-store/src/index_val_kernel.rs` | atom | `pedradb-store/layers.rs` |
| prefix | `pedradb-core/src/prefix.rs` | atom | `pedradb-store/lib.rs`, `pedradb-store/layers.rs` |
| changelog | `pedradb-core/src/changelog_kernel.rs` | close | `pedradb-core/db.rs` |
| range_covers | `pedradb-core/src/merge.rs` | model | `pedradb-core/merge.rs`, `pedradb-core/db.rs` |
| stream_cursor | `pedradb-stream/src/cursor_kernel.rs` | close | `pedradb-stream/lib.rs` |
| bearer | `pedradb-http/src/auth_kernel.rs` | atom | `pedradb-http/lib.rs` |
| content_length | `pedradb-http/src/cl_kernel.rs` | close | `pedradb-http/lib.rs` |
| fail_closed | `pedradb-http/src/fail_closed.rs` | close | `pedradb-http/lib.rs` |
| form_plus | `pedradb-http/src/form_kernel.rs` | atom | `pedradb-http/lib.rs` |
| origin_path | `pedradb-http/src/path_kernel.rs` | atom | `pedradb-http/lib.rs` |
| isolated | `pedradb-fold/src/isolated_kernel.rs` | atom | `pedradb-fold/follow.rs` |
| children | `montanha-fdb-recipes/src/children_kernel.rs` | atom | `montanha-fdb-recipes/lib.rs` |
| fields | `montanha-fdb-recipes/src/fields_kernel.rs` | atom | `montanha-fdb-recipes/lib.rs` |
| pack | `montanha-fdb-recipes/src/pack_kernel.rs` | close | `montanha-fdb-recipes/lib.rs` |
| journal_pin | `pedradb-journal/src/pin_kernel.rs` | close | `pedradb-journal/lib.rs` |
| ship_guard | `pedradb-replicate/src/ship_kernel.rs` | model | `pedradb-replicate/lib.rs` |
| bloom_header | `pedradb-core/src/bloom.rs` | close | `pedradb-core/bloom.rs` |
| bloom_insert | `pedradb-core/src/bloom.rs` | close | `pedradb-core/sst/table.rs` |
| bloom_may_contain | `pedradb-core/src/bloom.rs` | close | `pedradb-core/sst/table.rs` |
| scan_guard | `pedradb-core/src/sst/scan_kernel.rs` | model | `pedradb-core/sst/table.rs` |
| fold_range | `pedradb-fold/src/fold_kernel.rs` | close | `pedradb-fold/watch.rs`, `pedradb-fold/store.rs` |
| wal_recover | `pedradb-core/src/wal/recover_kernel.rs` | close | `pedradb-core/wal/reader.rs` |
| manifest_recover | `pedradb-core/src/manifest_kernel.rs` | close | `pedradb-core/db.rs` |
| reopen_outcome | `pedradb-core/src/wal/reopen_kernel.rs` | close | `pedradb-core/db.rs` |
| dictionary_link | `pedradb-core/src/wal/reopen_kernel.rs` | close | `pedradb-core/db.rs` |
| flush_decision | `pedradb-core/src/flush_kernel.rs` | close | `pedradb-core/db.rs` |
| compact_decision | `pedradb-core/src/compact_kernel.rs` | close | `pedradb-core/db.rs` |
| compact_retention | `pedradb-core/src/compact_kernel.rs` | close | `pedradb-core/merge.rs` |
| vlog_recover | `pedradb-core/src/vlog_gc_kernel.rs` | close | `pedradb-core/db.rs` |
| blob_gc_pick | `pedradb-core/src/vlog_gc_kernel.rs` | close | `pedradb-core/db.rs` |
| apply_step | `pedradb-raft/src/apply_kernel.rs` | close, single_artifact | `pedradb-raft/lib.rs` |
| grant_persist | `pedradb-raft/src/vote_kernel.rs` | close | `pedradb-raft/lib.rs` |
| **group_commit** | `pedradb-core/src/group_commit_kernel.rs` | close | `pedradb-core/concurrent.rs` |
| **group_fence** | `pedradb-core/src/group_commit_kernel.rs` | close | `pedradb-core/db.rs` |
| tx_glue | `pedradb-store/src/tx_glue_kernel.rs` | close, single_artifact | `pedradb-store/lib.rs` |
| **cf_family** | `pedradb-core/src/cf_kernel.rs` | model | `memtable.rs`, `sst/table.rs`, `db.rs`, `rocksdb-compat/lib.rs` |
| **visible_at** | `pedradb-core/src/merge.rs` | close | `merge.rs`, `memtable.rs`, `db.rs` |
| **ikey_pack** | `pedradb-core/src/key.rs` | close | `key.rs` |
| **write_record_count** | `pedradb-core/src/batch.rs` | close | `batch.rs` |
| **pin_gc** | `pedradb-core/src/compact_kernel.rs` | close | `db.rs` (`compact_reclaim`) |
| **wait_for_deadlock** | `rocksdb-compat/src/locktab.rs` | model | `locktab.rs` |
| **flush_publish** | `pedradb-core/src/flush_kernel.rs` | close | `db.rs` (`persist_manifest`) |
| **iter_window** | `rocksdb-compat/src/iter_kernel.rs` | close, single_artifact | `rocksdb-compat/lib.rs` (`page_forward` / `page_last_n`) |

Extratos Aeneas→Lean (segunda máquina — o próprio código de produção):
`WalRecover`/`Apply`/`ApplyKernel`/`Bloom` e agora `GroupCommitKernel` +
`GroupCommit.lean` (forma fechada universal + exemplos concretos pontuados
por `LawfulBEq.eq_of_beq (by native_decide)`, sem `sorry`).

## Residuais e donos ("tem algum desses que não tá nos RFCs?")

Inventário único + comparação seL4/IronFleet: **[RFC-0061](../rfc/0061-residuals-sel4-ironfleet.md)**
(`scripts/formal/residuals.json`, freeze no `--ci`). Pedra **não** é tão
robusto quanto seL4; está na mesma classe de *claim* (TCB escrito), não
na mesma classe de *garantia*.

| Residual (pergunta do usuário) | Dono | Mecanismo vivo | Resto honesto |
|---|---|---|---|
| Glue não-kernel (~69k LOC) | RFC-0056 (TCB freeze P2.5) + este mapa | freeze CI dos globs; lint de callers; oráculos DST/World | "verde à vista; zero não" — TCB declarado, nunca "sem bugs" |
| OS mentindo no fsync / torn writes reais | RFC-0052 P2 (caixa TCG/det_io; cross-ref 0059 P2.3 / 0057 P1.4) + kernels de recovery | P2.1 `tcg_world_smoke.sh` native=guest `trace_hash`; P2.3 `tcg_world_smoke_detio.sh` PRELOAD `libdet_io.so` drop_fsync (215 intercepts, same hash); P2.2 residual SSH | tmpfs + drop-return-0 ≠ prova de ECC/mídia; kernels de recovery cobrem o **modelo** |
| Merge group-commit (decisão) | **fechado 2026-08-24**: RFC-0057 P2.1/P2.2/P2.4 + RFC-0058 P2.1 | kernel provado (Verus 14/0 + Lean no-sorry) chamado por `validate_occ_batch`/`lone_commit`/fence; modo verificado `verified-v2` com merge ativo | — |
| Trajetória de cluster (termo/index entre exchanges) | **fechado**: RFC-0059 P2.2 | checker de trajetória + teste de mutante | escalas maiores continuam campanha, não teorema |
| Upgrade/membership mid-run | **fechado**: RFC-0059 P2.1 | schedules determinísticos de join/leave/reconfig/rollback no World | — |
| Paralelismo real do SO (interleavings fora do modelo PCT) | RFC-0057 P0.3/P0.4 + RFC-0052 P1.2 | work-stealing `run_swarm`, gate serial-vs-paralelo por `trace_hash`, TSan job | PCT d=2 cobre uma fatia do espaço de schedules — declarado, não total |
| io_uring ring | RFC-0058 P2.2 (gate documentado) | fora do modo verificado por contrato; full mode usa com `PosixFallback`; twin bloqueado em modelo de ring | sem promessa de prova do ring |
| **Field/hardware** (bit-rot fora de leitura, CPU errando, discos que somem) | **[RFC-0060](../rfc/0060-field-and-hardware-residuals.md) P0–P2 done** | `pedra verify` / `maintain --verify` (`verify_at_rest`); World `Action::BitFlip` + `silent_wrong==0` | CRC+scrub ≠ prova de ECC/mídia; TCG (RFC-0052 P2) ainda é o "modelo = hardware" |
| Reconfig out-of-band sem joint consensus | **P0 2026-08-27** RFC-0066: leave-joint (C-new only) after C-old,new commits; election still old∧new until leave (`election_after_committed_joint_still_requires_new_majority`). Enter-joint: 0063/0064 | Stateright leave = 0066 P1 | campaign, not a theorem |
| Apuração de voto / snapshot catch-up | **fechado 2026-08-24**: RFC-0059 P0.4c (seed 503976) | tally por candidato; reject de snapshot stale = hint não-match; label no applied | — |
| CHANGELOG lazy vs get após InstallSnapshot | **fechado 2026-08-24**: RFC-0059 P0.4c (seed 502514) | union per-key no lazy feed; oráculo de ressurreição exige get local ausente | — |

## Checksums (RFC-0060 P2.1)

Walked by `verify_at_rest` / `pedra verify`. On-read = the live get/open/recover
path. On-scrub = the at-rest walk. Holes are named, not hidden.

| Structure | Checksum | Verified-on-read | Verified-on-scrub | Hole? |
|---|---|---|---|---|
| SST file (`*.sst`) | CRC32C trailer of payload; each data block decoded | `SstTable::open_on` / get | yes (every data block) | no |
| Value log (`VALUES.vlog`, `*.blob`) | CRC32C per record (`len \| crc \| data`) | `read_ptr_on` fail-closed | yes (every record) | no |
| WAL (`CURRENT.log`) | CRC32C per physical record | `Wal::recover_on` fail-closed | yes (every record) | no |
| MANIFEST (`MANIFEST-NNNNNN`) | CRC32C trailer of payload | `manifest::load` | yes | no |
| `CURRENT` | optional CRC32C hex of named MANIFEST (RFC-0060 P2.15); one-line legacy | parse+crc (`manifest::load`) | parse + exists + crc if present (P2.3/P2.15); BitFlip inventory (P2.20) | legacy one-line has no CRC |
| Compaction/install temps (`*.tmp`, `CURRENT.tmp`, `MANIFEST-*.tmp`) | n/a (not inventory) | not read; open GCs orphans | leftover named FAIL (RFC-0060 P2.16) | leftover is FAIL, not CRC |
| History MANIFEST (`history/MANIFEST`) | CRC32C trailer (`PHST`, `crc_match_ok`, RFC-0086) | `HistoryTier::open` | yes (RFC-0060 P2.4 / RFC-0086) | no |
| History segments (`history/seg-*.hist`) | CRC32C per record (`crc_match_ok`, RFC-0087) | `walk_segment_records` fail-closed | yes (RFC-0060 P2.4 / RFC-0087) | no |
| History bloom (`history/seg-*.bloom`) | CRC32C of body (`PHB1`, `crc_match_ok`, RFC-0088); remote put resume `crc_match_ok` + byte-equal (RFC-0093) | fail-open on read (never prune; RFC-0088 P1.1/P2.2, RFC-0093 P2.2) | yes, fail-closed (RFC-0060 P2.4 / RFC-0088 P0) | no |
| `CHANGELOG` | CRC32C trailer of payload (`crc_match_ok`, RFC-0085) | rebuilt from WAL if missing (F33 quarantine) | yes when present (RFC-0060 P2.6 / RFC-0085 P1.1); open still Ok | named: cache, not source of truth |
| `CHANGELOG.corrupt` | same as CHANGELOG | F33 rename of poison cache | yes (RFC-0060 P2.24) | leftover quarantine is FAIL |
| `VALUES.vlog.adopt` | n/a (legacy marker) | open GCs | leftover named FAIL (P2.25) | leftover is FAIL, not CRC |
| `CHECKPOINT` (checkpoint dest only) | CRC32C trailer (`PDBCKP01`/`PDBCKP02`) | `read_checkpoint_meta` | yes when present (RFC-0060 P2.7) | no |
| Backup `CATALOG` | CRC32C trailer (`PDBCAT01`) | `BackupEngine::open` | yes (RFC-0060 P2.17) | no |
| Backup `wal/*.warch` | CRC32C trailer (`PDBWAR01`) | `read_warch` on PITR restore | yes (`verify_at_rest` P2.18 + `verify_wal_archive` P2.9) | no |
| Remote history (`seg-*-*.hist`) | CRC32C per record + content-addressed name (`crc_match_ok` + byte-equal resume, RFC-0092) | `walk_segment_records` on restore | `RemoteTier::verify` / `pedra archive verify` (RFC-0060 P2.11) | no |
| Remote `LATEST` pointer | hex CRC32C of named `MANIFEST-n` (`crc_match_ok`, RFC-0089) | parse+crc or generation walk-back (P1.2: never serve named older) | `archive verify` names mismatch (RFC-0060 P2.12 / RFC-0089) | torn pointer falls back |
| `LOCK` | none | n/a | skipped | n/a |
| Unrecognized leftover files | n/a | not read | named FAIL (RFC-0060 P2.27); dotfiles skipped | leftover is FAIL, not CRC |
| `CORRUPTLOG` | none (append-only TSV) | open counts lines (RFC-0038 D) | parse-walk (RFC-0060 P2.22) | no CRC; garbage is FAIL |
| Compat `CFREG` | optional `c:` CRC32C hex of prefix | `load_cf_registry` fail-closed | yes (RFC-0060 P2.26) | legacy without CRC still loads |

**Hardware residual (RFC-0060 P2.2 / LEDGER L50):** CRC+scrub prove that
bytes *we walk* still decode. They do not prove the drive's ECC, a bit that
flips after the last scrub, or a CPU that computes CRC wrong. Those stay
with the device and with RFC-0052 P2 (TCG/det_io) — "is the model the
hardware?" is a different question from "did this file's CRC match."

## Como este mapa se mantém vivo

O freeze (`pedra_formal --ci`) já falha se um kernel driftar do twin ou se um
caller deixar de chamar a entrada; **este doc** é o inventário humano e deve
ser re-medido quando: (a) um par entrar/sair do catálogo, (b) um residual
trocar de dono (ex.: TCG pousar → linha do fsync muda de "modelo" para
"hardware na caixa"), ou (c) the 0060 checksum table gains/loses a hole.
