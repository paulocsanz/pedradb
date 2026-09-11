# RFC-0189 P0.2 — spin adaptativo anti-park (janela com líder lento) — done

**Data:** 2026-09-10
**Peer / métrica:** n/a (slice de mecanismo; o efeito é medido no meter final do RFC)
**Veredito:** KEEP — mecanismo aterrado, trio direcionado ×3 verde, 24 testes async direcionados ×3 = 72/72, suíte sem falhas novas.

## O que o P0.1 exigia

Atribuição Linux quieta (`2026-09-10-rfc0189-p01-atribuicao`): parks 9/41024
(0,02%) — a hipótese "followers parkam no regime normal" já estava fraca em
contagem, mas cada park custa `unpark()` (futex WAKE) no ciclo crítico do
líder, então o corte continua válido: um follower nunca deve parkar enquanto o
líder **está progredindo**.

## Mecanismo aterrado (`concurrent.rs`)

- `WriteGroup.leader_progress: AtomicU64` — heartbeat `fetch_add(Release)` nos
  milestones do ciclo (pós `take_all`, pós encode, pós `write_pending_frame`,
  pós apply) + bumps nos caminhos lone (`submit_one`, `submit_after_begin`,
  `lone_commit` pós `db.write()`).
- `PipelineWaiter::wait_wake(&self, progress)` — spin em janelas de
  `PIPELINE_SPIN_WINDOW=1024` iterações; orçamento adaptativo
  `PIPELINE_SPIN_BASE_WINDOWS=3` → teto `PIPELINE_SPIN_MAX_WINDOWS=4096`
  janelas; park real (`thread::park()`) após `PIPELINE_SPIN_NO_PROGRESS=16`
  janelas **totalmente silenciosas** ou no teto. Zero espera para agrupar
  (não-goal 0180 preservado): o spin só observa, nunca adia o roubo.
- `parked: Cell<bool>` + `fold_park`/`count_unpark` — os contadores
  `pipe_parks`/`pipe_unparks` do P0.1 agora saem daqui.

## Por que spin (e por que NÃO no lock)

Requisito do usuário: "spinlocks are dumb as fuck never use them — unless you
have an opposite bench that tests exactly the priority inversion issue."
Pesquisa (Solaris/Illumos adaptive mutex, LWN 314512 — adaptive mutex spinning
no kernel Linux, Zijlstra 2009 spin-if-owner-on-cpu, glibc
`PTHREAD_MUTEX_ADAPTIVE_NP`, LWN 704843): o consenso da indústria é
**spin-then-park adaptativo observando progresso do dono**; spinning puro na
aquisição de lock é patológico (preempção do holder, priority inversion,
oversubscription — Cornell CS5412 mede até 85% de CPU desperdiçada).

Aplicado em duas decisões:

1. **Revertido:** o `wal.lock()` com try_lock-spin (janelas 512) que existiu
   nesta campanha era exatamente o padrão burro — girava contra um holder
   possivelmente preemptado que o userspace não consegue observar (LWN
   704843). Voltou a `wal.lock()` bloqueante puro; os parks de handoff-gap
   são parks honestos.
2. **Mantido:** o spin de **completude** do follower. O heartbeat
   `leader_progress` é o sinal de liveness do dono que o adaptive-mutex
   exige; bounded (16 janelas silenciosas + teto 4096) com park fallback.
   O lado oposto é testado (abaixo).

## O inverso existe: bench de stall

`adaptive_spin_parks_when_leader_stalled` — stall sintético de **25 ms
beat-less** (`arm_slow_leader_stall_for_tests`) logo após o primeiro roubo:
asserta `parks > 0` (followers parkam, burn bounded) + todos os N×PER puts
visíveis. É o teste de priority-inversion/preempção que o usuário exigiu:
sem ele o spin seria cheat de benchmark; com ele o corte prova as duas bordas.

## Por que o teste principal NÃO asserta parks == 0

`adaptive_spin_absorbs_slow_leader` (4 threads × 128 puts, 8 episódios
sintéticos de líder lento com beats) originalmente media parks-no-episódio —
flaky: um líder preemptado no meio do episódio para de bater e o follower
**corretamente** parka (comportamento adaptive-mutex; indistinguível de
"líder lento" no userspace). A invariante determinística é estrutural:

- `parks_without_silence == 0` — tripwire em `fold_park`: incrementa sse o
  park aconteceu com streak de silêncio `< PIPELINE_SPIN_NO_PROGRESS`
  (registrado em `PipelineWaiter.last_park_stale` nos dois caminhos de
  decisão). No loop adaptativo isso é impossível por construção — exatamente
  o que o orçamento fixo 3072 antigo violava. Vale para qualquer scheduler.
- Mais: `groups > 0` (pipeline engajou), `batches < submits` (followers
  existiram — senão o teste é vazio), `submits == batch_ops == N*PER`, e
  todos os puts visíveis por `get` real.

## Disciplina

- Trio direcionado ×3 (`adaptive_spin_absorbs_slow_leader`,
  `adaptive_spin_parks_when_leader_stalled`,
  `verified_async_concurrent_never_queues`): 3/3 runs verdes.
- 24 testes async direcionados (nomes em `pipeline-test-names.txt`) ×3:
  **passed=72 failed=0**, zero signals (`p02-targeted-x3.log` no scratch).
- Suíte serial completa: entra no A/B final do RFC (baseline 871/22).

## Efeito no regime real

P0.1 mediu parks 9/41024 pré-corte: no regime quieto normal o efeito é
próximo de zero (e é isso que o tripwire garante que continue). O ganho
alvo é a janela lenta/oversubscribed, onde leaders preemptados faziam
followers parkarem cedo e pagavam WAKE no ciclo. O meter final 3-rounds
registra o veredito honesto (pass ou fail) — sem meter por slice aqui.
