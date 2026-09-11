# 2026-09-10 — Por que a previsão do ciclo de write errou tanto (e o conserto do sistema de prever)

**Pedido:** "descubra por que a previsão errou tanto e corrija o sistema de prever".
**A previsão que errou:** `pedra scale-model write --fixture linux-quiet
--leaders 4 --guard 0` ⇒ `ticket_qps_hat=278 784`, `lock_wait_hat_ns=1537`
(`findings/2026-09-10-rfc0193-telem-ticket-view.md`).
**O medido:** perna isolada 1024×10k, guest p149b, 2026-09-10 03:32Z
(`findings/2026-09-09-overwrite-mc4-linux-p149b/serial.md`): QPS
87 262 / 115 452 / 103 096 (erro do hat **+2194 / +1414 / +1704‰**) e
`lock_wait` medido **16,7–18,5 µs** contra o hat de 1,54 µs (**×10,9–12,0**).

Nada aqui é cartaz; Darwin = DIAG; peer segue Rocks default `sync=false`.

## Causas-raiz nomeadas (com número)

1. **Conflação teto↔previsão (a maior).** `qps_from_cycle_ns = 1e9/(CS+wait)`
   é a taxa de **um** pipeline serial de líder; o QPS do bench é
   `L / latência_média_do_cliente` (Little, L=4). No pin quieto o próprio
   "teto" **fica abaixo do medido**: `1e9/(5520+2460) = 125 313` vs medido
   222 125 (**−435‰**) — os 4 clientes sobrepõem o trabalho fora-de-CS uns
   dos outros. O número nunca foi teto nem previsão: era incomparável.
2. **Termo de variabilidade ausente no wait.** `(L−1)/L·CS` assume serviço
   determinístico (Cs²=0). As caudas medidas (p50 7,7–8,5 µs, p99 446–551 µs;
   razão 52–65×) dão lognormal σ̂ = ln(p99/p50)/z99 ≈ **1,70–1,80**, scv
   **17,1–24,1**, amplificação Kingman do lado do serviço
   `(1+scv)/2 ≈ 9,1–12,6` — exatamente a ordem do erro ×11 do wait.
3. **Trabalho fora-de-fase ausente no ciclo.** Média/op r2 = 34 646 ns =
   wait 17 300 (**50%**) + CS_view 2 490 (**7%**) + fora-de-fase 14 856
   (**43%**) — cliente/framework/apply que o modelo de ciclo não via.
4. **Pernas comparadas fora do protocolo quieto canônico.** O 03:32Z **não**
   fez STOP/CONT do warm10 (protocolo canônico: `floor-cut-package`,
   `rfc0193-p05-meter-blocked`). O canário Rocks acusa r1/r3 carregadas
   (233k/201k < piso quieto ≳260k) e, pior, r2 "quieto pro Rocks" (328k)
   ainda assim Pedra lenta: 2,5 h depois, mesma célula/guest **com** STOP,
   `lock_wait=2,46 µs` e QPS 222 125 (`rfc0189-p01-atribuicao`) — fator
   **×7,0** visível só à Pedra, a classe de contaminador documentada em
   RFC-0176 P2.2 ("`p12_warm10.sh` left sleeping").
5. **Vintage do pin.** O hat empilhou dois cortes hipotéticos (`--guard 0` +
   vista ticket 0193) sobre o pin pre-0193 e comparou contra um binário
   outra época. Hipotético-sobre-hipotético vs medido-poluído soma os dois
   lados do erro.

## O conserto (kernel `write_cycle_kernel`, tier calibrado)

- `lognormal_mean_mult_permille(p50,p99)` — σ̂ = ln(p99/p50)/z99 (z99 =
  2,32635), multiplicador da média `e^{σ̂²/2}`, em aritmética **inteira**
  (Q=2^20; ln por série atanh; exp por Taylor com termos overflow-checked;
  cauda > 1024×p50 satura — guarda de misuse).
- `scv_permille_from_p50_p99` + `variability_amplification_permille` — o
  termo ausente nomeado como átomo.
- `calibrated_forecast(leaders, p50, p99, cut_shift, measured_qps)` —
  `mean_hat = p50·e^{σ̂²/2}`, `qps_hat = L·1e9/mean_hat`, `error_permille`
  (o P1.2 existente), e **ganho de corte projetado na média calibrada**
  (`qps_after_cut`), não no ciclo-teto.
- Tier teto **rotulado** (`tier=ceiling`) na renderização e no WRITEPHASE:
  bound estrutural pra ranquear corte, nunca QPS cartaz.

## Validação (inteiros do kernel, mesmas 3 pernas)

| perna | p50/p99 µs | qps medido | hat antigo (err) | hat calibrado (err) |
|---|---|---:|---:|---:|
| r1 | 8,5 / 551 | 87 262 | 278 784 (**+2194‰**) | 94 288 (**+80‰**) |
| r2 | 7,7 / 502 | 115 452 | 278 784 (**+1414‰**) | 103 626 (**−102‰**) |
| r3 | 8,5 / 446 | 103 096 | 278 784 (**+1704‰**) | 110 518 (**+71‰**) |

Redução de erro de 14–21×. Consistência interna da lognormal nas próprias
pernas: média/p50 medida 4,50–5,39 vs `e^{σ̂²/2}` 4,26–5,01 (±12%).

**Ganho de corte corrigido:** o corte 0193 (1 240 ns de CS) na distribuição
r2 dá **+33‰** (107 066), não os +60,2% citados de teto-contra-teto. O ganho
de teto só se realiza se o CS dominar o ciclo — as pernas mostram que ele é
7% da média.

## Resíduos honestos

- O tier calibrado é **auto-calibrado pela própria perna** (prediz a média
  a partir da forma p50/p99); o uso composicional (cut-shift) foi validado
  só nas 3 pernas 03:32Z (±102‰). Não é oráculo de outra máquina.
- A perna quieta 06:09Z **não capturou p50/p99** (o split não grava) —
  reabrir: capturar p50/p99 em toda perna WRITEPHASE.
- O fator ×7 de protocolo não é previsto — o forecast presume perna no
  protocolo canônico (STOP/CONT warm10, canário Rocks ≥ 260k).
- CS_view das pernas 03:32Z é incerta (binário entre pins: mem 0,55–0,77
  não bate com o split fino de 06:09Z); por isso o tier calibrado **não
  depende de CS** — só p50/p99/L.

## Provas

- Kernel + testes: `crates/pedradb-core/src/write_cycle_kernel.rs`
  (`rfc0192_calibrated_*`, `rfc0192_ceiling_*`, `rfc0192_ln_fp_*`).
- CLI: `pedra scale-model write … --p50-ns N --p99-ns N [--cut-shift-ns N]
  [--measured-qps N]` + teste de igualdade em
  `crates/pedradb-cli/tests/scale_model.rs`.
- Serial `-p pedradb-core --lib -- --test-threads=1`: 939 pass / 22 fail;
  conjunto = baseline 21 + `rfc0167_l0_stall_parks_until_worker_drains`,
  que **passava no baseline** e falha **com o meu diff revertido
  cirurgicamente** (regressão de edição concorrente in-flight na árvore
  compartilhada, não deste corte; caminho L0-stall intocado aqui).
- Binário CLI: 2 execuções idênticas (`$S/write-calibrated-run{1,2}.txt`).
- RFC-0192: fatia P0.4 (tier calibrado + rótulo de teto) —
  `docs/rfc/0192-write-cycle-forecast.md`.
