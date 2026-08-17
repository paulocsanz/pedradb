# RFC: 0038 — Recuperação de WAL corrompido: fail-stop × self-heal (DECISÃO EM ABERTO)

**Status:** draft (parked — decisão pendente; este doc é a pesquisa + espaço de opções)
**Updated:** 2026-08-16
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

## Proposed Solution (decisão pendente — não implementar antes de escolher)

Opções em mesa:

- **A. Manter fail-stop + heal por réplica (CRDB-style).** Sem mudança no core; Montanha
  majority re-heala o nó. Custo: exige replicação pronta; single-node segue exigindo humano.
- **B. Primitiva PIT contabilizada (a favorita da pesquisa).** `OpenOptions::wal_recovery`:
  `FailStop` (default, atual) | `PointInTime { on_loss: callback/return }` — trunca na
  primeira corrupção, **quarentena o sufixo** (arquivo à parte, não apaga), e devolve
  relatório: bytes/records descartados, intervalo de seq perdido, offset. Perda vira
  evento explícito, nunca silêncio. Réplica/PITR usa o relatório para re-aplicar.
- **C. Ferramenta de salvage offline** (analógico `ldb repair`/`RepairDB`): binário `pedra
  salvage` best-effort, nunca no caminho de open.

B não muda default nem quebra G8 (perda declarada ≠ perda silenciosa). A e B se compõem.

## Delivery slices (mandatory)

### P0 — pesquisa verificada (este doc)

- [x] **P0.1** Fontes primárias (RocksDB options/log_reader/db_impl_open; Pebble reader/open)
  + excertos persistidos — status: `done`
- [x] **P0.2** Espaço de opções + correção do registro anterior — status: `done`

### P1 — decisão (bloqueado em escolha do dono)

- [ ] **P1.1** Decidir A/B/C (ou composição B+A) — status: `todo` (parked)
- [ ] **P1.2** Se B: desenho da API (`wal_recovery`, tipo do relatório, quarentena) — status: `todo`

### P2 — implementação (só após P1)

- [ ] **P2.1** Modo escolhido + painel EXPLODE re-run (toda injeção → fail-stop **ou**
  truncamento contabilizado; nunca divergência silenciosa) — status: `todo`
- [ ] **P2.2** Integração heal: réplica/PITR consumindo o relatório — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | fontes primárias persistidas | done | findings/wal-recovery-sources-20260816 | 2026-08-16 |
| P0.2 | p0 | opções + correção de registro | done | este doc | 2026-08-16 |
| P1.1 | p1 | decidir A/B/C | todo | — | 2026-08-16 |
| P1.2 | p1 | desenho da API do modo B | todo | — | 2026-08-16 |
| P2.1 | p2 | implementar + EXPLODE | todo | — | 2026-08-16 |
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
