# RFC: 0048 — Hunt fixes: compat read-path, OCC read-set, memtable scan, WAL fail-closed

**Status:** in-progress
**Updated:** 2026-08-22

## Background

- Adversarial hunt 2026-08-21 (fan-out de 7 agentes + prova dois estados no
  harness `pedradb-dst`) encontrou 7 famílias de bugs: 4 no `rocksdb-compat`
  (cache TLS cross-instância, direção/bounds do raw iterator, snapshot sem
  pino de GC + refill que engole `Err`), 3 no `pedradb-core` (commit OCC
  read-only pula read-set, scan do memtable ignora range tombstone, header
  Zero do WAL engole bloco) + 1 oracle RED pré-existente (resync do WAL que
  re-ancora engole a região danificada — `FailClosed` virava `SilentWrong`).
- Cada bug tem prova dois estados: teste falha no AS-IS, passa após o fix de
  causa única (`pedradb-dst/harness/tests/{compat_hunt,core_hunt}.rs` +
  oracle `wal_crc_flip_is_fail_stop_or_clean`).
- Detalhes por achado: `determinismo/pedradb-dst/findings/F165..F171-*.md`.

## Problems This Solves

- **Problem:** duas instâncias de DB na mesma thread partilham respostas de cache TLS (valor/contagem de A servidos por B).
- **Problem:** raw iterator anda para trás no `next()` pós-seek reverso e `prev()` ignora lower bound — divergência do rust-rocksdb.
- **Problem:** `DB::snapshot()` do compat é sequência nua; com `auto_reclaim` default (perfil Rocks) o snapshot vivo morre (`SnapshotTooOld`) e o iterator termina truncado em silêncio.
- **Problem:** commit OCC read-only devolve `Ok(())` sem validar o read-set — conflito e `SnapshotTooOld` indetetáveis (contrato do próprio `occ.rs`).
- **Problem:** `MemTable::range_snapshot` ignora range tombstones (`get` = Deleted, scan emite a chave).
- **Problem:** header Zero+len0 do WAL descarta o resto do bloco sem validação; e o resync que re-ancora perde a região danificada em silêncio — ambos violam o contrato fail-closed do `WalRecovery`.

## Proposed Solution

- Identidade global por instância no cache TLS do compat (`cache_epoch_base`).
- Flag de direção + clamp de bounds no raw iterator; `SnapshotPin` no `Snapshot`; `status()` no iterator (refill registra `Err`).
- Validar read-set no commit OCC vazio (espelha `lone_commit`).
- Filtrar `range_deleted` no scan do memtable.
- Fail-closed nos dois buracos do WAL: Zero-header com cauda não-zero (só a alignment fresca; dentro de walk é lixo do próprio walk) e resync re-ancorado reportado (`resync_origin`) — `FailClosed` recusa, `PointInTime` reporta `kind: "resync"`.

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)
- [x] **P0.1** compat: cache TLS com identidade por instância (F165) — status: `done`
- [x] **P0.2** compat: direção/bounds do raw iterator (F166) — status: `done`
- [x] **P0.3** compat: `SnapshotPin` + `status()` no iterator (F167) — status: `done`
- [x] **P0.4** core: commit OCC read-only valida read-set (F168, incl. paridade do teste compat `txn_snapshot_hides_later_writes`) — status: `done`
- [x] **P0.5** core: `range_snapshot` filtra range tombstones (F169) — status: `done`
- [x] **P0.6** core: WAL Zero-header fail-closed a alignment fresca (F170; torn tails não regrediram) — status: `done`
- [x] **P0.7** core: WAL resync re-ancorado reportado; `FailClosed` recusa (F171; oracle RED verde + `point_in_time_reports_resync_reanchor`) — status: `done`

### P1 — next wave (depends on P0 or clearly deferrable)
- [x] **P1.1** Reparo de WAL mid-log: rewrite do WAL a partir dos records recuperados (hoje o PIT reporta mas o dano permanece; reopen fail-closed recusa para sempre) — status: `done` (2026-08-22; PIT com kind `resync` reescreve o WAL e o reopen fail-closed volta limpo — `point_in_time_reports_resync_reanchor`)
- [x] **P1.2** Jornal do `resync`/Zero-header no `CORRUPTLOG` com offset exato do início do dano (o `resync_origin` já existe; falta o consumo no `escalate_or_fail` usar kind próprio) — status: `done` (2026-08-22; kinds `resync` + `zero_header`, `CoreError::WalZeroHeader` tipado em vez de string-match — `zero_header_journals_and_pit_reports`)
- [x] **P1.3** Residuais do fan-out com prova pendente — status: `done` (F172 k7, F173 k8 no wave anterior; F174 fold×materialize k9, F175 checkpoint history tier k10 e F176 archive Err swallow k11 provados e corrigidos neste wave — cada um dois estados + ficha própria)
- [x] **P1.4** Paridade Rocks a documentar: `scan_count`/`raw_iterator_opt` fora do read-set OCC (F168-adjacente; decidir política e escrever no doc do txn) — status: `done` (doc-only; política guard-key documentada no header de `txn.rs` e nos métodos — tracking de scans fica como P2 se um caller precisar)

### P2 — later / polish
- [x] **P2.1** Fuzz de framing do WAL com o painel `explode_choices` + os dois novos kinds (`ZeroHeaderTail`, resync re-anchor) no sweep de corrupção — status: `done` (2026-08-22; `ZeroTail` + `ForgeZeroHeaderAlive` no painel, sweep afirma FailStop tipado `WalZeroHeader` e o re-anchor `resync_origin`; dentes verificados neutralizando o check F170: sweep falha `expected FailStop, got Ok([])`)
- [x] **P2.2** `blocks_overlapping_range` simétrico com `blocks_for_point` sob user-key split (leitor defender-se do writer futuro) — status: `done` (2026-08-22; partição `< s` + guarda `hi >= s`; prova unitária dois estados com índice hand-made, controles sem split idênticos)

### P1.5 — wave 2 "ache mais" (2026-08-22; superfícies não caçadas: cache, occ/concurrent, compat txn/shape, feed, manifest+WAL rotate, rewrite/merge)
- [x] **W1** core: `rewrite_ssts` derrubava tombstone em rewrite PARCIAL (gate `to_level == MAX_LSM_LEVEL` irrelevante) — F177, fix `bottommost = input cobre todas as SSTs` — status: `done`
- [x] **W2** core: `AnswerCache` FIFO fantasma do `invalidate` (capacidade mente + evicção prematura da reinserida) — F178, pareamento por época — status: `done`
- [x] **W3** compat: `scan_count` sem read-your-own-writes — F179, `staged_entries` + overlay — status: `done`
- [x] **W4** compat: `set_snapshot` no-op no raw iterator — F180, `ReadOptions.snap` — status: `done`
- [x] **W5** compat: seeks reversos param em cima do upper exclusivo — F181, `step_prev_past_upper` — status: `done`
- [x] **W6** core: WAL rotate trunca inode vivo com frame async pendente (reopen FailStop c/ L0+MANIFEST íntegros) — F182, drain antes do `create_on` — status: `done`
- [x] **W7** core: feed lazy descarta chaves flushadas (curto-circuito WAL) — F183, união CHANGELOG+WAL por cutoff de seq (+ hardening sort B2 latente) — status: `done`
- [x] **W8** core: `mem_layers` fold-antes-de-pending inverte first-hit do `lookup` (get stalado × scan; afeta fluxo sequencial) — F184, swap das cadeias — status: `done`
- Não-prováveis classificados (sem fix forçado): MANIFEST off-lock×CURRENT (interleave), CAS×mem aplicada (Env stall), leader panic (Env panic), rollback F173 no `compact_for_reads` (fault env), compat txn unpinned sob auto_reclaim, codec CF process-local, `Manifest::decode` with_capacity remoto (LOW).

### W3 — wave 3 "vai" (2026-08-22; backlog dois-estados + manifest pipeline, vlog/feed, compat CF/batch)
- [x] **W3.1** compat: codec CF process-local → registro persistido `CFREG` (frozen + reconcile fail-closed) — F185, c10/c10b — status: `done`
- [x] **W3.2** compat: `Transaction` sem pin de version-GC sob `auto_reclaim` — F186, c11 — status: `done`
- [x] **W3.3** core: snapshot off-lock de MANIFEST obsoleto regride CURRENT e deleta o manifest novo — F187, época monotônica + gate, k17 — status: `done`
- [x] **W3.4** core: valor honesto colidindo com mágica `VLG` sniffado como ponteiro — F188, escape `0x01` no chokepoint, k18 — status: `done`
- [x] **W3.5** core: vlog GC × caches sem espelho SST — F189 **REFUTADO** (3 rotas; retired espelha SSTs, parked é fechado por gate `mem_is_empty_for_rotate`; k20 guarda a invariante do gate) — status: `done` (dead end registrado)
- [x] **W3.6** core: feed não-lazy serve ponteiro VLG cru — F190, `resolve_feed_entries` na borda, k21 — status: `done`
- [x] **W3.7** compat: adicionar CF a DB default-only vaza `cf\0…` no scan default — F191, recusa fail-closed, c12 — status: `done`
- [x] **W3.8** compat: `open_default` não cria diretório (paridade rust-rocksdb) — F192, c13 — status: `done`
- [x] **W3.9** compat: `flush_wal` no-op sob `set_sync(false)` — F193, `inner.sync()`, c14 (fault env) — status: `done`
- [x] **W3.10** core: `compact_for_reads` sem undo F173 (GC falho commitado pelo próximo flush) — F194, k23 — status: `done`
- [x] **W3.11** core: `sync_dir` entre rename do MANIFEST e swing do CURRENT — F195, hardening aplicado (prova NEEDS-CRASH-ENV) — status: `done` (hardening)
- [x] **W3.12** sst/table "residual F167" — **REFUTADO** por leitura (tombstones coletados antes do salto; salto ⟺ sem pontos na janela) + diferencial mecânico k22 (150 seeds × 5 APIs de leitura em acordo) — status: `done` (dead end registrado)
- Abertos: manifest#3 (sync_dir Err pós-swing do CURRENT → memória atrás do disco; crash-window, precisa rename-as-commit+fence), history LOW (`Manifest::decode` with_capacity remoto) — como na wave 2.

### W4 — wave 4 "vai" (2026-08-22; fechamento do backlog: manifest pós-commit, write-group panic, point-cache fill, history remoto)
- [x] **W4.1** core: `sync_dir` Err DEPOIS do swing do CURRENT fazia undo de um commit já em disco (reopen quebra) — F196, Err tipado `ManifestCommittedUnsynced` + fence nos quatro sítios, k24 (RED: reopen quebrado AS-IS; ctl count=9 undo limpo) — status: `done`
- [x] **W4.2** core: pânico do leader do write-group deixa `leader_active` preso — todo writer futuro trava no recv() — F197, `catch_unwind` em `lead()` (libera grupo + Err pendentes + fence + `resume_unwind`); `FaultKind::Panic` no sim; k25 (RED: watchdog 10s; fix verde 0,05s) — status: `done`
- [x] **W4.3** core: fill do point-cache sem revalidar `published_seq` perde a invalidação de um publish concorrente (get serve v1 após put v2 Ok) — F198, recheck no fill + `invalidate_read_answers` ANTES do CAS do publish (vetor transitório do duplo-check OCC fechado pela reordem; prova dele NEEDS-INTERLEAVE-HOOK); k26 determinístico via `K26Env` (gates sync-WAL/read-vlog; SST decodifica inteiro no flush, leitura por get é a do `VALUES.vlog`) — status: `done`
- [x] **W4.4** core: `Manifest::decode` aloca `n×104B` antes de validar corpo (manifest remoto de 36 B reserva ~446 GB; morte por abort/OOM em overcommit estrito) — F199, rejeição `n > (body_len-28)/36` pré-alocação; k27 = guarda local / RED em host Linux (malloc ≤1 TB aceito como VA neste macOS — sonda documentada) — status: `done` (NEEDS-LINUX-ENV)
- Backlog da wave 2 encerrado: manifest#3 ✅ F196, concurrent#4 ✅ F197, concurrent#3 ✅ F198, history LOW ✅ F199.

### W5 — wave 5 "vai" (2026-08-22; superfícies novas: fold-GC do a28637a, pedradb-io-uring, tx/occ via subagentes)
- [x] **W5.1** core: `gc_below_floor` aplicava regra por-chave a RANGE tombstone — "superseded na própria start key" não decide um tombstone (esconde versões de OUTRAS chaves; versões mais velhas sobrevivem ao GC e SSTs de baixo podem ter dado coberto) → tombstone vivo derrubado, `get` ressuscita valor deletado — F200, drain do fold-GC conserva `RangeDeletion` (só bottommost compact descarta); k28 RED→GREEN + ctl (sem supersede / GC off) — status: `done`
- [x] **W5.2** core: `compact_reclaim` + `auto_gc_floor` usavam `oldest_pinned_sequence().unwrap_or(last_sequence())` — cegos ao `occ_registry` que o core `begin_occ` alimenta → snapshot de TX OCC aberta varrido (SnapshotTooOld) nos caminhos manual E auto — F201, `ConcurrentDb::from_db` compartilha o Arc do registry no `Db`; floors fazem `min(pin, oldest bound occ)`; k29/k29b RED (via stash)→GREEN + ctl — status: `done`
- [x] **W5.3** io-uring: `Seek for IoUringFile` delegava ao offset do KERNEL e o gravava no cursor sombra — fd O_APPEND de `open_append` abre em offset 0 mesmo com bytes → `WalWriter::new` via `stream_position()` calculava `block_offset=0` em TODO reopen: registro Full cruza a fronteira de 32 KiB e o reader fail-closed o rejeita (no Linux uring o pwrite em `pos`=0 ainda SOBRESCREVE o WAL do byte 0 — NEEDS-LINUX-ENV) — F202, `Seek` resolve contra o cursor sombra (`Start`/`Current` locais, `End` delegado); k30 contrato (0 vs 5) + k30b vítima WAL (k2 perdido) + ctl StdEnv — status: `done`
- [x] **W5.4** io-uring: `completion().next()` às cegas adota CQE órfão de operação cujo `submit_and_wait` falhou (ex. EINTR pós-submit) — resultado errado atribuído à op seguinte (avanço duplo do cursor, falso Ok de sync) — F203, `wait_tagged_cqe` casa `user_data` (0x77/0x5f/0xd1) e drena órfãos; compilação do caminho Linux verificada por `cargo check --target x86_64-unknown-linux-gnu` — status: `done` (NEEDS-LINUX-ENV, precedente F199/F195)
- [x] **W5.5** io-uring: tags **constantes** por opcode (F203) ainda deixavam o CQE órfão da *mesma* opcode ser o resultado da op seguinte (falso Ok de `fsync` / comprimento de write errado) — U1, `next_user_data` único + harvest no Err de submit; kernel `cqe_kernel.rs` (RED as-is vs unique; corre em macOS); caminho Linux via `submit_sqe` — status: `done` (NEEDS-LINUX-ENV para o ring real)

### W6 — wave 6 "vai" (2026-08-22; checkpoint × persistência concorrente + caches de leitura — candidatos C1–C3 do backlog comprovados)
- [x] **W6.1** core: `CountCache::record_dirty` early-out com map vazio descartava o publish que aterrissava entre o seq do leitor e o insert dele (ambos sob READ lock) → entrada pré-write validava para sempre (`get` valida só contra o dirty-log) — F204, remoção do early-out; k31 RED→GREEN + k31ctl — status: `done`
- [x] **W6.2** core: `ConcurrentDb::create_checkpoint` sem `persist_lock` → persist off-lock completa swing+GC de MANIFESTs entre a cópia do CURRENT e a listagem do checkpoint → destino reabre com `CorruptManifest` (backup inútil, cópia retorna Ok) — F205, `persist_lock` no wrapper (flush_lock→persist_lock→write); k32 RED→GREEN + k32ctl; cause-check isolado — status: `done`
- [x] **W6.3** core: cópia do WAL do checkpoint sem quiescer o pipeline — grupo in-flight (L0 acked órfão do CURRENT velho + WAL sem frame) OU frame async acked em buffer userspace → chave acked some do restore — F206, `self.wal.lock().flush()?` antes da cópia (mutex espera o grupo; flush empurra o buffer); k33+k34 RED→GREEN + ctl; porte alternativo (drain de `commit_inflight`) descartado por redundância — status: `done`
### W7 — wave 7 (2026-08-22/23; caches de leitura + floor de GC + change-feed)
- [x] **W7.1** core: fill do `last_prefix_cache` sem revalidar `published_seq` (furo F198 em versão prefixo; backlog C4) — publish concorrente sob READ lock limpa o cache no meio do walk, o fill pós-clear grava resposta pré-publish com o gen novo e ela valida até o próximo write; `last_under_user_prefix` serve `None`/chave velha após put Ok — F207, recheck `published_seq == snapshot` antes do insert; k35 RED 5/5 → GREEN 3/3 (steering: writer parado pós-fsync no sync do WAL + progresso observável via `read_probe`), k35ctl verde nos dois lados — status: `done`
- [x] **W7.2** core: floor de GC sem pins conta seq não-publicado (fan-out read-path #1) — `compact_reclaim`/`auto_gc_floor` caem em `last_sequence()` que inclui write aplicado-mas-não-publicado (janela off-lock do write-group); `raise_earliest_readable` sobe acima do `published_seq` e `count_in_range` no snapshot visível corrente falha `SnapshotTooOld` transitório — F211, cap `last_sequence().min(visible_sequence())` nos dois sítios (sem perdas: `for_oldest_snapshot` GC preserva versões > floor); k36 RED → GREEN + k36ctl + cause-check isolado — status: `done`
- [x] **W7.3** core: `ConcurrentDb::flush` rotaciona o WAL sem persistir o CHANGELOG com interval 0 (fan-out history #2) — o tail de persist da `Db::flush` (db.rs 3826) não existe no pipeline concorrente; a união F183 do feed vivo fica só o tail do WAL (chave flushed some do feed com `get` normal) e bare-drop/crash + reopen cai no short-circuit de `lazy_feed_entries` (5653) — F212, tail `persist_changelog_after_explicit_flush` no flush concorrente; k37 RED (vivo + reopen) → GREEN + k37ctl (close persiste) + cause-check — status: `done`
- [x] **W7.4** core: `commit_async_ops` nunca estende o change-log não-lazy (fan-out history #3) — todo write `no_sync` da face `ConcurrentDb` publica e fica visível via `get` mas o evento não entra no feed; o próximo commit sync persiste o CHANGELOG sem ele e o rebuild do open pula `sequence <= feed_max` — perda durável — F213, espelho do extend de `commit_ops_with` (6078) após o append WAL; k38 RED (vivo + reopen) → GREEN + k38ctl (sync alimenta) + cause-check — status: `done`

- Backlog wave 8 registrado no LEDGER (exploradores read-path/history): veneno
  TableCache/BlockCache no undo de `compact_for_reads` (#2), `enforce_cap`
  ignora `occ_registry_floor` (#4), watermark `keep_only_latest` na janela
  off-lock (limite deliberado do F211), feed reopen last-per-key com WAL
  vivo (#1, checar spec), `try_scan_at` vs `get_at` (#5, checar spec),
  decode_block fail-open (#6/#3 LOW), remote AlreadyPresent len+crc (#7 LOW).
- Nota de numeração: F208–F210 da tabela pertencem à wave 8 (unsafe) da
  sessão paralela; os desta wave foram renumerados F211–F213 na
  consolidação (sequência única do LEDGER).
- Backlog #2 (enforce_cap × occ_registry_floor) **REFUTADO** por derivação:
  os floors F201/F211 mantêm `archive_floor ≤ snapshot` OCC aberto e a
  fronteira de `ensure_snapshot_readable` é `<` estrito; guarda de
  regressão `k39_defense_cap_gc_holds_open_occ_snapshot` verde nos dois
  lados (dead-end F214 no LEDGER).

- Refutados na onda (dead ends com análise): fadvise overflow→"até EOF" (equivalente ao clampe; único caller passa u32), trunc `as u32` em write >4 GiB (escrita parcial é contrato de `Write`), EINTR no fdatasync (propagar Err é correto); tx/occ: skip de commit com `last_sequence()==snap` defendido pela write lock.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | compat TLS cache cross-instância (F165) | done | hunks em `rocksdb-compat/src/lib.rs` | 2026-08-21 |
| P0.2 | p0 | compat raw iterator direção/bounds (F166) | done | `rocksdb-compat/src/shape.rs` | 2026-08-21 |
| P0.3 | p0 | compat snapshot pino + status (F167) | done | `rocksdb-compat/src/lib.rs` | 2026-08-21 |
| P0.4 | p0 | OCC read-only valida read-set (F168) | done | `pedradb-core/src/occ.rs` + teste compat | 2026-08-21 |
| P0.5 | p0 | memtable scan range tombstone (F169) | done | `pedradb-core/src/memtable.rs` | 2026-08-21 |
| P0.6 | p0 | WAL Zero-header fail-closed (F170) | done | `wal/reader.rs` + `wal/recover_kernel.rs` | 2026-08-21 |
| P0.7 | p0 | WAL resync re-ancorado reportado (F171) | done | `wal/reader.rs` + `wal/mod.rs` + `db.rs` | 2026-08-21 |
| P1.1 | p1 | WAL rewrite p/ dano mid-log (reopen FC limpo) | done | `db.rs` + `point_in_time_reports_resync_reanchor` | 2026-08-22 |
| P1.2 | p1 | kinds `resync`/`zero_header` no CORRUPTLOG | done | `error.rs`/`wal/reader.rs`/`db.rs` + `zero_header_journals_and_pit_reports` | 2026-08-22 |
| P1.3 | p1 | residuais do fan-out | done | F172 (k7), F173 (k8), F174 (k9), F175 (k10), F176 (k11) | 2026-08-22 |
| P1.4 | p1 | paridade Rocks: scans no read-set OCC | done | doc em `rocksdb-compat/src/txn.rs` | 2026-08-22 |
| P2.1 | p2 | fuzz framing WAL (novos kinds) | done | `wal/recover_choose.rs` + `tests/recover_choose.rs` | 2026-08-22 |
| P2.2 | p2 | simetria blocks_for_point/range | done | `sst/table.rs` + teste unitário dois estados | 2026-08-22 |
| W1 | p1.5 | rewrite parcial conserva tombstone (F177) | done | `db.rs` + `merge.rs`; prova k12 (artefato temporário) | 2026-08-22 |
| W2 | p1.5 | FIFO do AnswerCache sem fantasma (F178) | done | `cache.rs`; k13 | 2026-08-22 |
| W3 | p1.5 | scan_count com own-writes (F179) | done | `occ.rs` + compat `txn.rs`; c7 | 2026-08-22 |
| W4 | p1.5 | set_snapshot efetivo no raw iterator (F180) | done | compat `shape.rs` + `lib.rs`; c8 | 2026-08-22 |
| W5 | p1.5 | seeks reversos vs upper exclusivo (F181) | done | compat `shape.rs`; c9 | 2026-08-22 |
| W6 | p1.5 | rotate WAL drena pendente antes do truncate (F182) | done | `db.rs` (`rotate_wal_now`); k14 | 2026-08-22 |
| W7 | p1.5 | feed lazy une CHANGELOG+WAL (F183; +B2 latente) | done | `db.rs` (`lazy_feed_entries`, `collect_feed_from_live`); k15 | 2026-08-22 |
| W8 | p1.5 | mem_layers pending-antes-do-fold (F184) | done | `db.rs` (`mem_layers`); k16 | 2026-08-22 |
| W3.1 | w3 | CFREG persistido p/ codec CF (F185) | done | compat `lib.rs`; c10/c10b/c10ctl | 2026-08-22 |
| W3.2 | w3 | pin de GC na Transaction (F186) | done | compat `txn.rs`; c11/c11ctl | 2026-08-22 |
| W3.3 | w3 | época de MANIFEST off-lock (F187) | done | `db.rs` (`take_manifest_persist`/`ManifestPersist`); k17 | 2026-08-22 |
| W3.4 | w3 | escape de colisão VLG inline (F188) | done | `db.rs` (`escape_inline_value`); k18 | 2026-08-22 |
| W3.5 | w3 | vlog GC × parked/retired — REFUTADO | done | dead end + guarda k20 do gate | 2026-08-22 |
| W3.6 | w3 | feed resolve ponteiros na borda (F190) | done | `db.rs` (`resolve_feed_entries`); k21 | 2026-08-22 |
| W3.7 | w3 | recusa add-CF em default raw (F191) | done | compat `lib.rs`; c12 | 2026-08-22 |
| W3.8 | w3 | open_default cria diretório (F192) | done | compat `lib.rs` + `txn.rs`; c13 | 2026-08-22 |
| W3.9 | w3 | flush_wal fsynca de verdade (F193) | done | compat `lib.rs`; c14 (FailingEnv) | 2026-08-22 |
| W3.10 | w3 | undo F194 no compact_for_reads | done | `db.rs`; k23 (Rename count=3) | 2026-08-22 |
| W3.11 | w3 | sync_dir antes do swing CURRENT (F195) | done | `manifest.rs` (hardening; NEEDS-CRASH-ENV) | 2026-08-22 |
| W3.12 | w3 | sst overlaps residual — REFUTADO | done | diferencial k22 (scan×get×count×prefix) | 2026-08-22 |
| W4.1 | w4 | commit pós-swing cercado (F196) | done | `manifest.rs`/`db.rs`/`concurrent.rs`/`error.rs`/compat; k24 | 2026-08-22 |
| W4.2 | w4 | leader panic não engalha writes (F197) | done | `concurrent.rs` + `FaultKind::Panic` no sim; k25 | 2026-08-22 |
| W4.3 | w4 | point-cache fill revalida published (F198) | done | `db.rs` (`get` fill + `publish_sequence`); k26 | 2026-08-22 |
| W4.4 | w4 | contagem do manifest remoto validada (F199) | done | `history.rs`; k27 guarda (NEEDS-LINUX-ENV) | 2026-08-22 |
| W5.1 | w5 | fold-GC conserva range tombstone (F200) | done | `memtable.rs`; k28 + ctl | 2026-08-22 |
| W5.2 | w5 | reclaim/auto-GC honra occ_registry (F201) | done | `db.rs`/`concurrent.rs`; k29/k29b + ctl | 2026-08-22 |
| W5.3 | w5 | io-uring Seek pelo cursor sombra (F202) | done | `pedradb-io-uring/src/lib.rs`; k30/k30b + ctl StdEnv | 2026-08-22 |
| W5.4 | w5 | CQE casado por user_data (F203) | done | `pedradb-io-uring/src/lib.rs` (NEEDS-LINUX-ENV) | 2026-08-22 |
| W5.5 | w5 | CQE tag único + harvest no submit Err (U1) | done | `cqe_kernel.rs` + `submit_sqe`; `forbid` CLI/alias-smoke; SAFETY posix | 2026-08-22 |
| W6.1 | w6 | count-cache rastreia dirty com map vazio (F204) | done | `cache.rs`; k31 + k31ctl | 2026-08-22 |
| W6.2 | w6 | checkpoint serializa com persist off-lock (F205) | done | `concurrent.rs` (`persist_lock`); k32 + k32ctl | 2026-08-22 |
| W6.3 | w6 | checkpoint drena WAL antes de copiar (F206) | done | `db.rs` (`wal.lock().flush()`); k33/k34 + ctl | 2026-08-22 |
| W7.1 | w7 | last_prefix_cache fill revalida published (F207) | done | `db.rs`; k35 + k35ctl | 2026-08-22 |
| W7.2 | w7 | floor de GC capped no published (F211) | done | `db.rs` (`compact_reclaim`, `auto_gc_floor`); k36 + k36ctl | 2026-08-23 |
| W7.3 | w7 | flush concorrente persiste CHANGELOG (F212) | done | `concurrent.rs` + helper `db.rs`; k37 + k37ctl | 2026-08-23 |
| W7.4 | w7 | commit async alimenta feed não-lazy (F213) | done | `db.rs` (`commit_async_ops`); k38 + k38ctl | 2026-08-23 |

## Acceptance Criteria

- **Tests:** `pedradb-core --lib` (395 passando — incluindo `point_in_time_reports_resync_reanchor`, `zero_header_journals_and_pit_reports`, `torn_tail_*`, theorem do recover kernel, sweep `explode` com os kinds novos e a simetria de blocos), `pedradb-io-uring` (env + `cqe_kernel` U1 as-is vs unique), `rocksdb-compat` (41+7), harness `compat_hunt` (**21**, c1..c14 + controles) + `core_hunt` (**64**, k1..k39 + controles + diferencial k22; k39 é defesa/controle do backlog refutado #2) + oracle `wal_crc_flip_is_fail_stop_or_clean` — todos verdes com os fixes; os de hunt falham sem eles (F196–F198, F200–F207, F211–F213 demonstrados RED→GREEN; F199/F203 guardas/fixes condicionados a host Linux conforme fichas).
- **Telemetry / Analytics:** none — correção de corretude; o `CORRUPTLOG` (RFC-0038) recebe eventos `resync` e `zero_header` (P1.2).
- **Documentation:** este RFC + fichas F165–F213 em `determinismo/pedradb-dst/findings/` (+ dead ends F189/sst/io-uring registrados) + LEDGER do hunt 2026-08-21/22/23 (waves 1–7 + backlog wave 8) + patches `core-hunt-20260822.patch` (waves 1–3), `core-hunt-20260822-wave4.patch` (delta da wave 4), `core-hunt-20260822-wave5.patch` (delta da wave 5) e `core-hunt-20260822-wave6.patch` (delta da wave 6) e `core-hunt-20260822-wave7.patch` (delta da wave 7: F207/F211/F212/F213, 6 hunks, apply/reverse-apply verificados).
- **Screenshots:** backend-only.

## Out of scope

- Fix dos residuais não provados do fan-out (P1.3 lista; cada um exige prova dois estados própria antes).
- Redesign do recovery PIT para reparar dano mid-log in-place (P1.1 é o rewrite mínimo).
- LSM bottom-level tombstone drop (merge.rs) — dependente de caller, sem prova.
