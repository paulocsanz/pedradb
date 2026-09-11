# Forense do wipe 2026-09-10 — o que "aterrissou" mas nunca foi commitado

**Data:** 2026-09-11 | **Método:** `git grep <símbolo> --all -- 'crates/**'`
(qualquer commit alcançável), leitura direta do tree vivo, correlação com
o snapshot `a5ccc131` ("trabalho in-flight da sessão paralela") e com os
commits dos meters (f7b2c20f, b2b0295b, b959428a).

## O que aconteceu

Sessões de 2026-09-10 aterrissaram e **verificaram em working tree** uma
série de cortes (findings e testes da época existem e eram verdes), mas o
código nunca chegou a ser commitado. O `git reset --hard` da sessão
paralela (2026-09-10 23:49) destruiu esses working trees; o snapshot
`a5ccc131` preservou apenas o estado PÓS-reset. Arquivos kernel PUROS que
tinham sido commitados individualmente sobreviveram no disco — alguns sem
a declaração `pub mod` no `lib.rs` (código morto, não compilado).

## Auditoria por símbolo (tree vivo 2026-09-11 vs histórico --all)

| corte | símbolo do seam | tree vivo | algum commit? | conclusão |
|---|---|---|---|---|
| 0193 P0.1–P0.4 (pwrite off-lock) | `write_all_at*`, `reserve_frame`, `positional_writes` | **0 hits** | **nunca em `crates/`** | **wiped-pre-commit** — findings (`2026-09-10-rfc0193-p01-wal-before/` etc.) documentam execução e verificação; o código não existe em nenhum commit |
| 0189 P0.3 (publish skip fills==0) | `read_tls_fills` | 0 hits | nunca | wiped-pre-commit |
| 0190 P0.3 (lanes) | `lane_histogram` (0192 P2.1) | 0 hits | nunca | wiped-pre-commit (verificar lanes restantes caso reabra) |
| 0194 P0.1–P0.3 (leftover advise) | `leftover_advise` | 0 hits fora do kernel | `leftover_page_kernel.rs` commitado, **`pub mod` jamais** | kernel vivo em disco, morto; wiring wiped |
| 0195 P0.1–P0.3 (scan readahead) | `scan_readahead` | 19 hits SÓ dentro do kernel | `scan_readahead_kernel.rs` commitado, **`pub mod` jamais** | idem |
| 0197 P0.3 (CLI ratio) | `scale-model ratio` | subcomando inexistente | `ratio_curve_kernel.rs` commitado, **`pub mod` jamais** | idem |
| 0192 P0.2 (WRITEPHASE fine-slices) | 8 fatias enc/wr/guard/mlock/mins/grp | tree tem as 6 fatias RFC-0159 | nunca | wiped-pre-commit |
| 0192 P0.3 (CLI `scale-model write`) | subcomando | inexistente | nunca | wiped-pre-commit |
| 0192 P0.1/P0.4 (kernel + tier) | `write_cycle_kernel.rs` | **vivo** (declarado; 19/19 testes verdes hoje; `calibrated_forecast`, `qps_hat_error_permille`, vista ticket) | sim (`b959428a`) | sobreviveu |
| 0201 P0.3 (merge eixo cliente) | `client_axis_kernel` + concurrent.rs | **vivo** | sim (`f7b2c20f`) | sobreviveu |
| 0201 P0.1 (clamp) | clamp no drain | **vivo** | sim (`b2b0295b`) | sobreviveu |

## Implicação 1 — atribuição dos meters p201q/p201r2 (CORREÇÃO)

A imagem p201r2 foi construída da árvore viva em 2026-09-11 07:51
(f7b2c20f + b2b0295b + b959428a) — **SEM o seam do 0193** (verificado:
`write_all_at*` ausente de f7b2c20f e de todo o histórico). Logo:

- **apply_mc4 1,0859× e kvrocks_set_mc50 1,678× foram pagos pelo merge
  por eixo de cliente 0201 P0.3 + clamp P0.1, sem qualquer código 0193.**
- Atribuições anteriores a "off-lock 0193 + 0201" (escritas antes desta
  forense) estão corrigidas nos RFCs 0196/0193 e no `docs/status.md`
  neste mesmo commit.

## Implicação 2 — verificação de CLI do plano do ciclo

`pedra scale-model ratio` (superfície 0197) não existe no CLI vivo —
verificação de determinismo 2× substituída pela superfície `scale-model
--keys/--ram` (0176, viva). Registrada como desvio no plan.md.

## Implicação 3 — o buraco async 1-op é ainda maior

Sem o off-lock 0193, TODO o caminho de escrita WAL paga `write()` por op
dentro do wal lock — o staging do RFC-0209 ataca exatamente esse ponto,
independente do ticket. O re-land do 0193 não é exigido por nenhuma
célula não paga do board atual (mc50/apply pagas; 1-op → 0209); reabre
se uma célula futura voltar a depender dele.

## Lição (já em prática neste ciclo)

Commit defensivo imediato por caminhos explícitos após CADA onda; nunca
confiar em "aterrissado" de sessão anterior sem `git grep` do símbolo no
histórico — findings e RFCs descrevem o working tree da data, não o repo.
