# RFC-0307: Linearidade de Leitura, Visibilidade do Point-Cache e Glue Final do Ponto

**Status:** Implementação em andamento (P0 e P1 concluídos)
**Data:** 3 de Outubro de 2026
**Base:** `b7fcde6b` (RFC-0306 P2 publicado)
**Alvo:** vencer ou empatar em TODAS as métricas do `snapshot-bench` em TODAS as
escalas, com curvas lineares em #tabelas, e fechar o furo de visibilidade G2.

---

## 1. Contexto e Diagnóstico (evidência já medida)

Placar em `b7fcde6b` (100M/250M): hydrate **3,36 M/s** (1,94× Rocks), settle, miss-out
(43-111ns) e get_loop ganhos; miss-in caiu −54% (8,97→4,15µs @886 tabelas) e hit-warm
−76% (156→38µs) com os flat bounds. Restam quatro pontos, todos com causa atribuída:

1. **G2 (bloqueante de correção):** get devolve `None`/valor-velho para put-Ok recente,
   transitório, ~1-2/10 no oráculo diferencial **sem workers**.
   **CAUSA-RAIZ ENCONTRADA E CORRIGIDA (P0):** `check_run` no caminho de leitura dos
   runs bulk chamava `range_deleted(key, 0, &range_tombs)` — **seq do ponto = 0**
   hardcoded. Qualquer range-tombstone cobrindo a chave escondia o valor achado no
   run, **mesmo sendo mais velho que ele**. Um put que revive chave range-deletada
   lia `None` enquanto vivesse num run latched/parked/encoding; curava no settle
   (SST compara seqs reais). Flaky porque o latch só engaja em streak de batches
   ascendentes (ordem de HashMap → composição). Eliminação por camada completa em
   `findings/2026-10-02-stale-get-visible-seq-lag.md`.
2. **prefix_scan degrada com escala** (170→240→330µs de 100M→500M; Rocks flat ~169):
   `LevelRunStream` bisecciona HIs no padrão-chase; todo get de ponto proba a tabela
   do ladder (`tables/get 1,985`).
3. **get_hit/get_loop ~1,5-3µs atrás do fjall**: glue por get (mutex global do
   point-cache, hash duplo, cópia de saída, probe do ladder) — **expandido no RFC-0308**.
4. **cold hit ~20-25% atrás do Rocks**: first-touch (partição de bloom lazy) — RFC-0308 Pilar E.

## 2. Pilares

### P0 — Visibilidade (CONCLUÍDO)

1. `note_dirty_points` de batch fat/range agora **limpa o point-cache na hora**
   (a flag `point_cache_reset` diferida deixava janela em threads reais).
2. **Fix da causa-raiz**: `BulkRun::lookup_with_seq` novo; `check_run` compara
   tombstones contra a **seq real da entrada**. Teste de regressão determinístico
   `bulk_run_put_after_range_delete_is_visible` (puts ascendentes engajam o latch;
   shape-guard de `bulk_live_bytes > 0`) — **verificado por mutação**: falha contra
   a forma bugada, passa com o fix.
3. Armadilha permanente: check `PRE-MERGE STALE READ` no oráculo diferencial +
   branch counters (`POINT_GET_BRANCH`) + `PEDRA_ORACLE_NO_BG` (zero workers real).

### P1 — prefix_scan linear (CONCLUÍDO)

1. `SstRun::disjoint_his_flat: Option<Vec<Bound32>>` — HIs em slots contíguos
   (mesma exatidão dos LOs; a forma estrita erra para "manter um arquivo a mais",
   o bounds check por tabela continua como fail-safe).
2. `LevelRunStream::new` bisecciona o array plano de HIs (fallback chase quando None).
3. LevelRunStream toma `&[usize]` emprestado (sem clone por scan).

### P2/P3 — Glue do ponto e cold hit

Absorvidos e expandidos pelo **RFC-0308** (pilares C, D, A, B, E).

## 3. Não-negociáveis

- Zero knobs; P0 antes de qualquer corte de perf; suítes completas + oráculo por
  pilar; guardas de recall (fail-safes de bounds como oráculo).
