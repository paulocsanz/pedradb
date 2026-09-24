---
name: endgoal
description: >
  Disciplina de endgoal do PedraDB: três endgoals sequenciais (EG1
  velocidade vs RocksDB default + fjall em todos os benches; EG2
  verificação formal de tudo; EG3 testagem dinâmica concorrente
  extensa), cada um com escada e % próprio em docs/TRAJETORIA.md. Só se
  passa ao próximo quando o anterior está 100%. Use quando o usuário
  pedir "endgoal", "qual o %", "pra onde vamos", "% implementado",
  /endgoal, ou ao planejar qualquer fire (otimizar/caminho-sel4/audit).
---

# endgoal (pedradb — 3 endgoals sequenciais)

Os endgoals, as escadas e os % vivem em
[`docs/TRAJETORIA.md`](../../../docs/TRAJETORIA.md). Padrão herdado do
centro/fonte (carteira: `~/.grok/skills/endgoal`), estendido a pedido do
operador (2026-09-21) para **três endgoals com gate sequencial e %
separados**.

1. **EG1 — Velocidade:** mais rápido que RocksDB **default**
   (`ROCKS_PARITY_SYNC=0`) e que **fjall** em todos os benchmarks
   existentes e nos novos que cozinharmos.
2. **EG2 — Formal:** verificar formalmente tudo (relativo ao TCB
   publicado).
3. **EG3 — Dinâmico concorrente extenso:** mais garantias por campanha
   dinâmica em profundidade/escala.

## Ordem do turno

1. Ler o bloco dos três endgoals, as linhas **Progresso** e as recusas na
   TRAJETORIA.
2. Conferir o **gate sequencial**: fire novo só paga fatia do endgoal
   ativo (o primeiro com % < 100). Exceções: ratchet same-fire de
   verificação em fire de engine (RFC-0222 P0.7, regra do otimizar —
   obrigatório, não é "trabalhar no EG2 cedo"); manutenção de gate verde;
   pedido explícito do operador.
3. Conferir que o corte planejado é degrau da escada do endgoal ativo
   (ou lateral nomeado). Corte sem degrau: registrar na escada primeiro.
4. Ao land: marcar `done` na escada **e** refazer a linha Progresso
   daquele endgoal — no mesmo change do fire.
5. Reportar sempre: "land <degrau>; EG<k> <X>% → <Y>%; próximo impago
   <degrau>; prova-termo <o que falta>". Sob /grind, reportar os três %.
6. Endgoal muda por pedido do operador ⇒ versão nova datada no
   **Histórico** + linha na memória do workspace; nunca edição silenciosa.

## % por endgoal (fórmula mecânica)

`piso(100 × (fatias done + ½ × fatias doing) / total de fatias DAQUELA escada)`

- Denominador é **por endgoal** (hoje: EG1=13, EG2=9, EG3=15) e vive na
  própria escada; shape/fatia nova muda o denominador no mesmo change.
- `done`=1, `doing`=½, `todo`=0; teto **C** registrado = done (estado
  terminal com mecanismo datado; não é win).
- Nunca arredondar para cima; nunca calcular no olho — refazer a fórmula.
- Os três % são independentes; o gate sequencial define qual é a frente
  ativa. "Qual o % do endgoal?" = os três + a frente ativa.

## Não

- Pagar fatia de EG(n+1) com EG(n) < 100% (fora as exceções do turno 2).
- Inventar endgoal, editar sem pedido, ou dois endgoals concorrentes
  dentro de um mesmo EG (o doc canônico é único).
- Marcar degrau `done` sem change correspondente; % refeito no olho.
- Chamar de win: peer `sync=true`, G1 1c write-per-op (teto fd),
  fjall como `compat_over_rocksdb`, Rocks colapsado, Darwin como cartaz
  (AGENTS.md + otimizar valem para toda a escada EG1).
- Criar dívida de verificação para "focar no EG1" — o ratchet same-fire
  é inegociável em fire que toca fn de kernel.
- Tratar relatório/status como land (report-only is a failure).
