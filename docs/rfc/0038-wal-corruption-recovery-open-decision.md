# RFC: 0038 — Recuperação de WAL corrompido: fail-stop × self-heal (DECISÃO EM ABERTO)

**Status:** in-progress (P0/P2.1 done; P1.1 decisão do dono pendente)
**Updated:** 2026-08-17
**Parents:** [0037](0037-apply-off-put-and-2x-pedra.md), garantias G1/G8; kernel [`recover_kernel`](../crates/pedradb-core/src/wal/recover_kernel.rs)

## Background — o que cada um faz (fontes primárias, verificadas 2026-08-16)

**Pedra hoje:** CRC mismatch no meio do WAL → `Db::open` retorna `Err(Crc)`, DB não sobe.
Torn tail / length / unknown-type → resync e prefixo válido é mantido (auto). Fonte:
[`recover_kernel.rs`](../crates/pedradb-core/src/wal/recover_kernel.rs) — "CRC and orphan **fail-stop**".
Excertos salvos: [findings/wal-recovery-sources-20260816](../findings/wal-recovery-sources-20260816/).

**RocksDB** (`facebook/rocksdb@main`, excertos em findings/): 4 modos, `WALRecoveryMode` em
`include/rocksdb/options.h`:

| modo | comportamento em corrupção mid-WAL | DB abre? |
|---|---|---|
| `kTolerateCorruptedTailRecords` (0x00) | reporta e **retorna status** | **não** |
| `kAbsoluteConsistency` (0x01) | idem, sem tolerância nenhuma | **não** |
| `kPointInTimeRecovery` (0x02) — **DEFAULT** | `stop_replay_for_corruption=true`, retorna `OK`: replay **para na corrupção** (ponto-no-tempo) | **sim** |
| `kSkipAnyCorruptedRecords` (0x03) | ignora tudo, "last ditch effort to recover data" | **sim** |

Mechanics (`db_impl_open.cc` `HandleNonOkStatusOrOldLogRecord` + `log_reader.cc`): o reader
reporta e tenta resync; a decisão de parar é da camada de open. Use-cases citados nos
próprios comentários: PIT para "disk controller cache... SSD without super capacitor";
skip-any para "recovery after a disaster". Detalhe revelador: com `paranoid_checks=false`,
o corruption **nem vira status** — só log; e o checksum continua ligado sempre porque
"corruptions cause entire commits to be skipped instead of propagating bad information
(like overly large sequence numbers)" — ou seja, o medo deles não é perder writes, é
**propagar estado furado** (silent wrong), igual ao nosso F4/F14.

**Pebble/CockroachDB** (`wal/reader.go`): reader é fail-stop explícito — "it seems safer to
be explicit and surface the corruption error here" — e `Open` não oferece salvage de WAL.
A filosofia CRDB: nó corrompido morre; a **replicação** (Raft) re-heala o dado.

**Correção de registro (G8):** afirmei antes em conversa que o default do RocksDB seria
`kTolerateCorruptedTailRecords` "pulando cauda". Duas partes erradas: o default é
`kPointInTimeRecovery` (options.h:1480), e `kTolerate...` **se recusa a abrir** em
corrupção mid-log. Este RFC carrega a versão verificada.

## A hipótese do usuário (para decidir)

> "olha se o rocksdb pula provavelmente é pq precisam pra manutencao em escala... duvido que
> seja sustentável ter DBs em escala assim, no MÁXIMO dar uma primitiva pra se auto corrigir
> com réplicas, redundância, PITR, que aí o nó consegue se auto-healar"

O que a pesquisa confirma/contra-argumenta:

- **A favor:** o engine mais deployado do mundo escolheu, como default, **abrir mesmo com
  corrupção** truncando no ponto (PIT). Em frota, pager-humano por bitflip não escala; e o
  modo "desastre" existe porque sempre precisam dele no pior dia.
- **Contra mudar o default às cegas:** RocksDB também tem os dois modos estritos; o PIT
  deles descarta silenciosamente (log-only) writes acked **depois** da corrupção — a classe
  de perda que o Pedra proíbe por padrão. F4/F14 existem porque "tolerar" mal implementado
  virou silent wrong quando invertemos a decisão.
- **Síntese plausível:** o sustentável em escala não é tolerância local permissiva; é
  **fail-stop + heal por redundância** (CRDB) **OU** **truncamento explícito e contabilizado**
  (um PIT com recibo, não silencioso). O que falta ao Pedra é a *primitiva* de opt-in.

## Problems This Solves

- **Problem:** nó único em frota com WAL corrompido exige intervenção humana (restore de
  backup/truncagem manual) — não escala; e não há primitiva para um nó se re-semar
  de réplica/PITR automaticamente.
- **Problem:** não existe modo de recuperação *contabilizado*: ou fail-stop, ou (inexistente)
  perda silenciosa — faltam os dois pontos intermediários honestos.

## Refinamento pós-discussão (2026-08-16) — o argumento "disco morrendo" invertido

Discussão com o dono derrubou o argumento mais fraco do fail-stop: *"bitrot anuncia disco
morrendo, então parar é feature"*. Se o disco está mesmo morrendo, (a) reportar não conserta
física nenhuma, (b) o `Db::open` fechado **bloqueia a evacuação** do prefixo são, (c) nenhum
modo local recupera o que ficou ilegível. Três classes de corrupção mid-WAL e por que o
evento único não discrimina:

| classe | causa | disco | resposta certa |
|---|---|---|---|
| cauda rasgada | power loss / cache não-durável | são | auto (já feito, kernel) |
| corrupto **isolado** | raio cósmico / controladora / bug | são | PIT contabilizado + quarentena |
| **progressiva** | mídia morrendo | ruim | **evacuar**, nunca "tolerar e seguir" |

A classe progressiva só se revela **no tempo** (eventos repetidos, EIO, SMART). Logo: a
decisão tolerar×parar não é modo de open, é **política de escalonamento com histórico**.
Opções atualizadas:

- **A.** fail-stop + heal por réplica (CRDB) — inalterada.
- **B1.** PIT contabilizado **servindo**: prefixo + quarentena + relatório; writes seguem. Risco: em disco progressivo, cada restart perde mais — só defensável com D.
- **B2.** PIT contabilizado **modo evacuação**: abre **read-only** (ou writes recusados), prefixo legível para o orquestrador drenar/replicar; nunca "seguir servindo às cegas".
- **C.** salvage offline — inalterada.
- **D. (novo) Journal de corrupção + escalonamento**: arquivo append-only por DB registrando todo evento (offset, intervalo de seq, bytes, modo, timestamp). Política: ≥N eventos ou qualquer EIO → open recusa **em qualquer modo** → evacuação. É o discriminador isolado×progressivo que o evento único não dá.

Síntese: **D é pré-requisito de qualquer modo tolerante**; B2 é o caminho de evacuação que o
fail-stop puro bloqueia; B1 só com D. A primitiva de heal (réplica re-semeando o intervalo de
seq do relatório; PITR para single-node) consome o mesmo relatório.

## Refinamento pós-implementação de D (2026-08-17) — o painel adversarial achou dois buracos

Implementar D (journal + escalada) expôs que o journal honesto exige distinguir **garbage do
próprio resync** de **corrupção real de disco**; sem isso, D briquaria DBs saudáveis:

1. **CRC durante um resync-walk é lixo do walk, não evidência de disco ruim.** Cauda rasgada
   cujo payload parcial contém uma janela (type,length) plausível fabrica um header falso
   durante o byte-walk de resync → CRC mismatch. No kernel antigo (CRC = fail-stop sempre,
   mesmo em walk), isso (a) derrubava um open que deveria auto-recuperar e (b) com D,
   **jornava** um evento "crc" — três crashes de rotina = `CorruptionEscalated` em DB são.
   Correção: CRC em **alinhamento limpo** (logo após registro válido) continua fail-stop
   **com journal** (G8 intacto); CRC **dentro do walk** continua andando e, no EOF, mantém o
   prefixo **sem journal** (semântica kEof do log_reader.cc: "assume the writer died in the
   middle. Don't report a corruption").
2. **Torn tail recuperado precisa ser truncado no open.** `Wal::append_on` anexava depois da
   região rasgada; o próximo open relia o lixo enterrado como se fossem registros → Crc em
   alinhamento limpo → fail-stop **com journal** (o crash de rotina nº 2 já jorna). Correção:
   `Db::open` agora trunca o WAL para `last_good_offset` (o fim do último registro CRC-válido
   recuperado) via `EnvFile::set_len` + `sync_data` antes de reabrir para append.

Achados verificados por teste end-to-end (`db::tests::torn_tail_does_not_journal`,
`db::tests::wal_crc_corruption_journals_then_escalates_then_recovers`): cauda rasgada abre,
serve o prefixo, escreve, reabre, **nunca cria `CORRUPTLOG`**; bitflip isolado fail-stoppa e
jorna em toda tentativa; o 3º evento recusa open em qualquer modo; WAL reparado/substituído
abre limpo (escalação nunca briqua diretório são).

## Delivery slices (mandatory)

### P0 — pesquisa verificada (este doc)

- [x] **P0.1** Fontes primárias (RocksDB options/log_reader/db_impl_open; Pebble reader/open)
  + excertos persistidos — status: `done`
- [x] **P0.2** Espaço de opções + correção do registro anterior — status: `done`

### P1 — decisão (bloqueado em escolha do dono)

- [ ] **P1.1** Decidir composição (candidata da discussão: **D + B2 primeiro**, B1 só depois de D medido em frota; A quando Montanha majority existir) — status: `todo` (parked)
- [ ] **P1.2** Desenho da API: `wal_recovery` (`FailStop` | `PointInTimeReported`), tipo do relatório de perda, quarentena (`WAL.corrupt-<n>`), formato do journal de corrupção (D) — status: `todo`

### P2 — implementação (só após P1)

- [x] **P2.1** D (journal + escalonamento) primeiro — status: `done` (2026-08-17)
  - `crates/pedradb-core/src/corrupt.rs`: journal append-only (`CORRUPTLOG`: ts, kind, offset),
    `escalate_or_fail` — só fail-stops jornam (crc, truncated_head); caudas rasgadas nunca.
  - `CoreError::CorruptionEscalated` no 3º evento (`CORRUPTION_ESCALATION_EVENTS`), recusa
    open em qualquer modo; reparo/substituição do WAL reseta (journal fica, contagem conta
    eventos — decisões de limpeza ficam para P1.2).
  - Refinamentos obrigatórios achados pelos testes (seção acima): CRC-em-walk não jorna;
    truncamento para `last_good_offset` no open.
  - Painel EXPLODE re-run fica para o modo escolhido em P1.1 (o painel de enumeração já
    existe em `wal::recover_choose`).
- [ ] **P2.2** B2 (evacuação read-only do prefixo) + heal: réplica/PITR consumindo o
  relatório — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | fontes primárias persistidas | done | findings/wal-recovery-sources-20260816 | 2026-08-16 |
| P0.2 | p0 | opções + correção de registro | done | este doc | 2026-08-16 |
| P1.1 | p1 | decidir A/B/C | todo | — | 2026-08-16 |
| P1.2 | p1 | desenho da API do modo B | todo | — | 2026-08-16 |
| P2.1 | p2 | implementar D (+ refinamentos) | done | corrupt.rs + kernel/reader/truncate + testes | 2026-08-17 |
| P2.2 | p2 | heal por réplica/PITR | todo | — | 2026-08-16 |

## Acceptance Criteria

- **Tests:** painel `explode_choices` inteiro sob o novo modo: cada corrupção injetada
  produz exatamente (fail-stop) ou (prefixo + relatório de perda exato); diferencial vs
  truncagem manual byte-a-byte; adversarial G4 sem editar asserção.
- **Telemetry:** relatório de recuperação (bytes, records, seq range, offset) no retorno/
  log de open; teste afirma os números.
- **Documentation:** este RFC atualizado com a decisão + nota de compatibilidade.
- **Screenshots:** none — backend-only.

## Out of scope

- Mudar o default para qualquer modo tolerante antes de P1.1 decidir.
- Tolerância que descarte dados sem contabilizar (perda silenciosa) — G8 permanente.
- Skip-any no caminho de open automático (fica, se existir, como ferramenta offline C).
