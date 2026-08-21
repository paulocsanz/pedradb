# RFC-0047: rocksdb-compat como substituto completo — perfil de retenção e de falha do RocksDB na face compat (kernel segue fail-closed)

**Status:** draft
**Updated:** 2026-08-21
**Parents:** [0046](0046-mvcc-history-tiering-s3.md) (retenção default),
[0045](0045-multi-writer-async-5x.md),
[rocksdb-vs-pedradb-guarantees.md](../rocksdb-vs-pedradb-guarantees.md) §4 (direção aceitável),
[certainty-vs-availability.md](../certainty-vs-availability.md) §5.2b,
[AGENTS.md](../../AGENTS.md) (colunas oficiais medem o produto)

## Background

- Posição: `rocksdb-compat` é a cara de drop-in (API rust-rocksdb-shaped).
  Para ser **substituto completo** (exceto compatibilidade binária), o
  comportamento *operacional* tem que bater com o que um usuário do RocksDB
  espera — sem que o kernel (`pedradb-core`) deixe de ser fail-closed.
- Estado atual (verificado no fonte, 2026-08-21):
  - **Retenção**: compat herda o F20 do core (`Options.auto_reclaim:
    false` default; mecanismo pin-aware existe desde d9abd6f). Hoje o
    substituto é API-compatível mas **não** é storage-profile-compatível:
    disco O(writes) + cliff de scan frio, onde o RocksDB compacta para
    ≈ live set + pins.
  - **Corrupção no meio do WAL**: core fail-stop (`CoreError::Crc` no
    `collect_all`); torn tail já vira prefixo limpo (mesma classe do
    Rocks). O RocksDB default (`kPointInTimeRecovery`) **para no ponto da
    corrupção e segue servindo** o prefixo.
  - **CORRUPTLOG** (RFC-0038, `corrupt.rs`): journal de eventos
    fail-stop; o 3º evento recusa o open **em qualquer modo** —
    `CorruptionEscalated` já tipificado no `error.rs`.
  - **fsync falho**: kernel cerca (`DurabilityFenced`, resultado incerto);
    o único caminho é close+reopen. RocksDB tem `DB::Resume()` e
    auto-resume para classe transitória (ENOSPC).
  - **Erro do compat**: `Error(String)` opaco — o host não distingue
    fence de corrupção de IO; não dá para programar política em cima.
  - `last_good_offset` já existe no reader (`wal/reader.rs`) — a âncora
    para recovery de prefixo com relatório.

## Problems This Solves

- **Problem:** trocar RocksDB por pedra-compat hoje muda o perfil de
  storage (disco cresce sem borne) e o perfil de falha (nó para em vez de
  servir o prefixo) — não é um substituto, é uma troca de contrato.
- **Problem:** a política de disponibilidade não pode morar no kernel
  (G2/G5: fail-closed é o piso); mas o host do drop-in é o compat, e ele
  hoje não tem nem erro tipado para decidir.
- **Problem:** perfis de falha e de retenção do RocksDB precisam de
  contraparte **tipada e reportada** — copiar `paranoid_checks=false`
  (escrever sem reconhecer incerteza) é proibido por doctrine.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok (default) | **divergência deliberada e documentada**: o substituto é durável por default (e mais rápido); `sync=false` permanece opt-in para quem quer a forma exata do Rocks |
| G2 | CRC fail-closed / never silent-wrong | kernel intocado; modos de recovery são **explícitos, tipados, com relatório do descartado**; nunca silent-wrong |
| G5 | fence em sync-fail | kernel intocado; recovery do fence **sempre reporta o range incerto** — nunca finge que sabe |
| — | CORRUPTLOG/RFC-0038 | escalonamento recusa open **em todo modo** (PointInTime incluso) |
| G6 | sem thread no core | intocada |
| G8/AGENTS | colunas oficiais = produto | bench pina a retenção medida **explicitamente** (env); nenhum shape sai (0043) |

## Proposed Solution

1. **Kernel intocado como piso; política na face compat.** Modos de
   recovery do WAL viram opção explícita do core (`FailClosed` default);
   o compat escolhe o perfil Rocks (`PointInTime`) e reporta o que
   descartou.
2. **Retention Rocks-shaped no compat por default**
   (`auto_reclaim: true`): compact derruba versões sem pin — o mesmo
   storage profile do RocksDB. F20 (história completa) vira opt-out
   explícito no compat; no core, o default segue a decisão do RFC-0046.
3. **Erro tipado no compat** (`Error::kind()`): o host programa política
   (fence / corrupção / escalada / IO) em vez de parsear string.
4. **`resume()`** (equivalente do `DB::Resume()`): close+replay+reopen
   assistido, com relatório tipado do range in-flight (resultado
   permanece incerto — reportado, não adivinhado).

## Delivery slices (mandatory)

### P0 — substituto com o mesmo perfil de storage e de open

- [ ] **P0.1** `Error::kind()` tipado no compat
      (`Fenced | Corruption{..} | CorruptionEscalated | Io | Invalid |
      Other`) preservando o Display; testes de mapeamento `CoreError →
      kind` — status: `todo`
- [ ] **P0.2** Core `OpenOptions::wal_recovery: FailClosed (default) |
      PointInTime`: no PointInTime, `collect_all` recupera o prefixo até
      `last_good_offset` e devolve `RecoveryReport` (offset da corrupção,
      registros/bytes descartados); **CORRUPTLOG ainda registra o evento**;
      escalada (3º evento) recusa o open mesmo em PointInTime. Compat
      default = PointInTime. Testes com as injeções de
      `wal/recover_choose.rs` (FlipCrc no meio → prefixo + report) —
      status: `todo`
- [ ] **P0.3** Compat `Options.auto_reclaim` default **`true`** (perfil
      Rocks: live set + pins); F20 = opt-out; bench pina a retenção
      medida via `ROCKS_PARITY_RETENTION=product|rocks` (default
      `product`) para que a virada de default do compat **não** mude
      silenciosamente as colunas oficiais; nota no README do bench —
      status: `todo`

### P1 — recover do fence (o "segue servindo" do lado do write)

- [ ] **P1.1** Core `recover_from_fence()` assistido (close+replay+reopen)
      devolvendo relatório tipado do range in-flight; compat
      `DB::resume()` em cima — status: `todo`
- [ ] **P1.2** Auto-resume só para classe transitória (ENOSPC-like) via
      seam `Host`; default `manual` para o resto (paridade de perfil com
      as severidades do Rocks, sem flag sem tipagem) — status: `todo`

### P2 — superfície de política

- [ ] **P2.1** Listeners/severidades no compat
      (equivalente `BackgroundErrorReason`-shaped) + tabela doc
      knob-RocksDB → comportamento-compat (inclusive as divergências) —
      status: `todo`
- [ ] **P2.2** `docs/usage.md`: seção "drop-in divergences" (sync default,
      escalada CORRUPTLOG, G1) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Error::kind() tipado no compat | todo | — | 2026-08-21 |
| P0.2 | p0 | wal_recovery PointInTime + report + escalada | todo | — | 2026-08-21 |
| P0.3 | p0 | compat auto_reclaim=true + bench pin | todo | — | 2026-08-21 |
| P1.1 | p1 | recover_from_fence + DB::resume() | todo | — | 2026-08-21 |
| P1.2 | p1 | auto-resume transitório via Host | todo | — | 2026-08-21 |
| P2.1 | p2 | listeners/severidades + tabela de knobs | todo | — | 2026-08-21 |
| P2.2 | p2 | docs "drop-in divergences" | todo | — | 2026-08-21 |

## Acceptance Criteria

- **Tests:** `compat_error_kinds_map_core`; `point_in_time_recovers_prefix_and_reports`
  (FlipCrc via recover_choose); `escalation_refuses_in_every_mode`
  (PointInTime incluso); `torn_tail_unchanged_prefix_only` (re-verde);
  `auto_reclaim_default_matches_rocks_profile`; `resume_after_fence_reports_uncertain_range`;
  suítes adversariais FailingEnv re-verdes **sem editar asserção**;
  crash-after-sync-put (G1) re-verde.
- **Telemetry:** `findings/rfc0047-*/` com o A/B do perfil (F20 vs
  auto_reclaim no mesmo workload de overwrite: disco final + scan frio) e
  o print do RecoveryReport; nenhuma coluna oficial muda sem o pin do
  bench (comparar JSON antes/depois do P0.3).
- **Documentation:** este RFC; update do §5.2b do
  `certainty-vs-availability.md`; `docs/usage.md` (P2.2).
- **Screenshots:** none — backend-only.

## Out of scope

- `sync=false` como default (nunca — é o produto, ver G1 acima).
- Continuar escrevendo após incerteza sem report (proibido por doctrine;
  não existe `paranoid_checks=false` sem tipagem aqui).
- Recuperação por arquivo/quarentena de SST individual (file-scope;
  follow-up além do WAL).
- Compatibilidade binária/FFI C (o substituto é em nível de API/fonte).
- Mudar peer oficial, pisos ou remover shapes (AGENTS/0041/0043).
