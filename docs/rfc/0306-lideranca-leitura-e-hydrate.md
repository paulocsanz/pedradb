# RFC-0306: Liderança em Leitura e Paridade de Hydrate

**Status:** Implementação em curso
**Data:** 30 de Setembro de 2026
**Alvo:** das 8 métricas oficiais do harness `snapshot_backends` a 100M, o Pedra perde 5 (hydrate 1,14 vs 1,38 M/s; get_hit 39,3 vs 36,1 µs; cold hit 61,4 vs 48,3 µs; cold miss 545 vs 337 ns; multi_get 4,73 vs 3,91 ms) e vence 3 (settle 7,8×; prefix_scan −19%; get_loop −12%). Este RFC ataca, em ordem de prioridade e razão custo/benefício: **P0** cold miss, **P1** hydrate, **P2** get_hit/cold hit. `multi_get` é handicap do harness (o adapter Rocks usa `MultiGet` batched; o nosso recebe `get` sequencial — byte-faithful do upstream, intocável).

---

## P0 — Cold miss: 545 ns → alvo ≤ 150 ns (vitória drástica ≥ 2×)

### Diagnóstico

O caminho de um `get_cf` que erra (`probe_miss`/`cold miss`) hoje paga, em ordem:

1. `tls_point_ids` — epoch + bucket FxHash do `KeyGenMap` (~60 ns);
2. `LAST_CF` TLS lookup — comparação de chave (~40 ns);
3. `codec.encode_with` — cópia `cf\0||key` em stack (~30 ns);
4. `ConcurrentDb::get` — read lock da Db, `visible_sequence`, clone do
   superversão publicado, mem/parked vazios, e só ENTÃO
   `fast_outside_sst_miss` (flag settled + envelope) responde `None`
   (~200–400 ns com cold misses de cache nos bounds `Bytes`).

O primitivo de rejeição rápida **já existe** (`fast_outside_sst_miss`,
contracto: `settled_sst_only` ⇒ mem/imm/parked vazios ⇒ chave fora do
envelope de toda família não existe), mas executa **depois** de 1–4.

### Correção

Rejeitar o miss **na primeira linha** do `get_cached`, em espaço de
usuário (sem encode), com envelope cacheado em TLS:

- O compat mantém por thread um envelope derivado:
  `(settled=true, por-CF: (pfx_len, user_lo, user_hi))`, validado por
  get contra `is_settled_sst_only` + `point_tls_epoch` + `read_cache_epoch`
  (3 loads atômicos; reconstrução só quando mudam).
- Chave crua fora de `[user_lo, user_hi]` da CF (comparação
  lexicográfica equivalente ao envelope codificado — pfx fixo por CF) ⇒
  `Ok(None)` imediato, contabilizado honestamente em `note_class_point`.
- Sem qualquer mudança de semântica: as mesmas premissas do
  `fast_outside_sst_miss` do core, apenas avaliadas antes e sem encode.

**Teeth:** teste de que (a) miss fora do envelope responde None sem tocar
o `inner` (contador de leituras não move), (b) qualquer write invalida o
cache (miss volta a consultar o caminho completo), (c) reopen/settle
reconstrói o envelope.

## P1 — Hydrate: 1,14 → alvo ≥ 1,40 M/s (paridade/vitória no teto do harness)

### Diagnóstico

Orçamento por batch de 1024 (caixa de bench): cliente do harness
~0,65 ms (comum aos três), engine Pedra ~0,26 ms, engine Rocks ~0,10 ms.
O teto do harness (engine=0) é ~1,6 M/s: **vitória drástica é
fisicamente impossível**; a vitória real é levar o engine a ≤ 0,10 ms.
Do custo engine, ~0,08–0,12 ms são 2.048 alocações por batch no
`WriteBatch::put_cf` (`Bytes::copy_from_slice` por chave e por valor).

### Correção — slab dual com retenção de capacidade e rollover

`WriteBatch` ganha dois slabs thread-local (`BytesMut`, 8 MiB): chaves e
valores. `put_cf`/`delete_cf`/`delete_range_cf` fazem
`extend + split_to(n).freeze()` — **zero alocação por operação** (um
refcount por view). Quando o slab enche, rola para um novo de 8 MiB; o
anterior sobrevive pelas views (refcount da alocação compartilhada do
`bytes`). Sem growth in-place com views pendentes: capacidade fixa +
rollover ⇒ nunca copia bytes acumulados.

**Por que o aa6d24e regrediu (0,57 M/s) e isto não:** aquele arena era
**por batch** — crescia do zero a cada apply (~18 reallocs copiando os
bytes acumulados, blocos grandes fora da fast path do allocator). Aqui a
capacidade é retida entre batches e o rollover nunca copia.

`PEDRA_ARENA_DISABLE=1` desliga (fallback ao caminho por-operação) para
A/B. **Teeth:** arena views sobrevivem a rollover e a growth; roundtrip
latched com arena; toggle desliga.

## P2 — get_hit/cold hit: verificação e ajuste

O caminho de hit herda o corte de custos do P0 (TLS ids ainda
necessários — são a invalidação do cache de resposta). O ganho restante
é disco: confirmar na caixa de bench que o `PlainBlockCache` (c80a105)
serve a forma do get_hit a 100M com o orçamento padrão, e ajustar
`PEDRA_PLAIN_BLOCK_BUDGET` se a diag indicar. Sem mudança de formato.

---

## Ordem de valiação

1. Microbench de miss no crate (settled + N SSTs) antes/depois do P0;
2. Suites: bloom, bulk, sst, compat (117), latcheds;
3. 100M na caixa de bench com `PEDRA_HYDRATE_DIAG=1` — `HYDRATEDIAG`
   atribui debt/lock/apply/publish do writer; `probe_miss`/`cold miss`
   medem o P0; `get_hit` o P2.

## Não-objetivos honestos

- Hydrate ≥ 1,3× Rocks: impossível sob o cliente compartilhado do
  harness (teto ~1,15×); a meta é vencer dentro do teto.
- multi_get: handicap do harness upstream (adapter byte-faithful).
- Qualquer mudança no formato SST/v6 ou na classe de durabilidade.
