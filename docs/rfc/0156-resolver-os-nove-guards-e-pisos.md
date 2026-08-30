# RFC: 0156 — Resolver os nove: guarda mecânica onde dá, piso nomeado onde não dá

**Status:** in-progress
**Updated:** 2026-08-30
**Parents:** [0155](0155-silent-wrong-fail-closed.md)

**Residual:** as nove linhas (`R-glue`, `R-group-glue`, `R-swarm-real`, `R-fsync-lie`, `R-unsafe-posix`, `R-unsafe-capi`, `R-es`, `R-crc`, `R-uring`) **continham publicadas**. Este RFC não apaga linha nenhuma. Onde a classe de silent-wrong é mecanicamente eliminável, ela ganha uma **guarda regressiva** (planta que falha se a classe voltar a entrar na árvore). Onde existe piso físico ou matemático, o piso é nomeado com o motivo — e a linha fica.

**Refused claims:** este RFC **não** alega “garantia total”, “sem bugs”, “perfeito”, “acabou” ou classe seL4. Campanha de 3 seeds não é ∀ TCP. PCT d=3 não é ∀ interleavings de lock do SO. CRC de 32 bits continua com colisão por pombal (`never_floor`). fsync-lie é piso do dispositivo. Liveness sem os axiomas ES-1..3 continua refutada pelo modelo.

## Background

O 0155 fechou o tabuleiro de recusa: cada um dos nove tem kernel fail-closed + planta **ou** linha publicada. “Resolver” cada ponto, honestamente, significa uma de duas coisas:

1. **Eliminação mecânica da classe**: o padrão que permite o silent-wrong não pode mais entrar na árvore sem quebrar um teste nomeado (guarda regressiva).
2. **Piso nomeado**: existe prova/argumento de que a eliminação total é fisicamente impossível do userspace ou matematicamente impossível; a linha residual fica com a guarda existente.

Estado por ponto antes deste RFC:

| Ponto | Já existia | Falta para “resolver” |
|---|---|---|
| R-unsafe-posix | `fdatasync_rc_ok` + gate em cada site | guarda: nenhum site FFI novo entra sem gate |
| R-unsafe-capi | `c_len_admitted` → LIMIT nos sites | sweep de fronteira em **toda** export com len |
| R-uring | `cqe_kernel` (tag única, `cqe_act`, F208) | sweep de sequência: leftover em toda posição nunca dá false-Ok |
| R-swarm-real | retry kernel + 1 seed | campanha multi-seed no REAL (ainda não ∀) |
| R-group-glue | PCT d=2 default + planta d=2 | campanha d=3 explícita (default segue 2) |
| R-glue | `claim_zero_glue` + freeze | extração de `db.rs` segue fora de escopo — piso |
| R-fsync-lie | `media_durable_admitted` false | userspace não prova a mídia — piso |
| R-crc | mismatch → Reject em SST/vlog/MANIFEST/WAL | colisão de 32 bits é pombal — piso (`never_floor`) |
| R-es | `liveness_admitted` exige ES-1..3; modelo refuta | eventualidade sem axioma é refutada — piso |

## Problems This Solves

- **Problem:** os gates do posix/capi eram corretos no commit, mas nada impedia um site novo sem gate entrar silenciosamente.
- **Problem:** a evidência TCP era 1 seed; campanha amplia sem virar ∀.
- **Problem:** PCT d=2 default congela o orçamento de campanha; d=3 explícito em teste não muda o default e exercita interleaving mais fundo.
- **Problem:** “resolver os nove” sem nomear os pisos viraria overclaim — os quatro pisos ficam escritos com o motivo.

## Proposed Solution

- **Guardas regressivas (P0):** scan-test estrutural no posix (todo site `unsafe` com FFI que devolve rc precisa de gate na mesma janela); sweep de fronteira de len na C ABI (`0, 1, cap, cap+1, usize::MAX` → LIMIT ou Ok, nunca pânico); sweep de sequência no modelo CQE (leftover em toda posição, nunca Take errado / false-Ok).
- **Campanhas (P0/P1):** 3 seeds REAL TCP com kernel e contabilidade de retry por seed; planta de profundidade 3 achada por PCT d=3 e não por d=2 no sweep medido (se a planta mostrar-se achantável a d≤2, o slice regride honestamente para “campanha d=3 sobre as plantas existentes” — medição decide, não a narrativa).
- **Pisos (P2):** os quatro pisos ficam nomeados no RFC e nas linhas; `never_floor` intocado; `db_rs_extracted=false`; default PCT continua 2.

## Delivery slices (mandatory)

### P0 — guardas mecânicas de classe

- [x] **P0.1** R-unsafe-posix: `posix_unsafe_rc_sites_all_gated` — todo site `unsafe {` chamando FFI de rc (`fdatasync`, `fcntl`, `fallocate`, `fsync`, `posix_fadvise`) tem gate (`posix_rc_to_io(rc)` / `rc == 0` / errno match) na janela da mesma expressão; falha se um site novo entrar sem gate — status: `done`
- [x] **P0.2** R-unsafe-capi: `capi_len_boundary_sweep_on_live_tx` — `transaction_set`/`transaction_get` com len ∈ {0, 1, cap, cap+1, usize::MAX}: oversize é `LIMIT` sem ler, admitido é Ok (TX nova por fronteira — orçamento cumulativo), null-com-len>0 é erro; nenhum pânico — status: `done`
- [x] **P0.3** R-uring: `cqe_leftover_sequence_never_false_ok` — 64 ops com tags únicas; leftover em toda posição é `Discard`; `submit_complete_act(_, false)` sempre `WaitMore`; false-Ok só existe no AS-IS (`cqe_res_ok_as_is`) — status: `done`

### P0 — campanha REAL

- [x] **P0.4** R-swarm-real: `l28_real_tcp_removed_campaign_seeds` — 3 seeds novas (0x0156/0x0157/0x0158_1E28) no cluster REAL; cada seed exige `napply=1`, kernels `l28_durability_ok`/`l28_tcp_napply_ok`, e `!l28_tcp_napply_retry_admitted(attempts, ok)`; campanha ≠ ∀ TCP (linha fica) — status: `done`

### P1 — profundidade de interleaving

- [x] **P1.1** R-group-glue: `planted_chain3_found_by_pct_d3` — planta de 3 tarefas com assinatura de cadeia-3 (`taken=3`, saldo −200); d=2 é estruturalmente incapaz (1 change point não estaciona dois) e mediu 0/256; d=3 mediu 8/16384; `forall_schedules_admitted(3)` segue false; **default PCT continua 2** — status: `done`

### P2 — pisos nomeados + registro

- [x] **P2.1** R-glue / R-fsync-lie / R-crc / R-es: pisos nomeados neste RFC (extração fora de escopo; mídia não é provável do userspace; pombal de 32 bits; eventualidade exige ES-1..3). `never_floor` com os seis ids; `db_rs_extracted=false` — status: `done`
- [x] **P2.2** close-text dos cinco fortalecidos (`R-unsafe-posix`, `R-unsafe-capi`, `R-uring`, `R-swarm-real`, `R-group-glue`) nomeia 0156 e a planta da guarda; linhas não apagadas; `--lint` 0 fail; freeze == live no fim — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | posix scan-test rc gateado | done | `posix_unsafe_rc_sites_all_gated` | 2026-08-30 |
| P0.2 | p0 | capi sweep de fronteira de len | done | `capi_len_boundary_sweep_on_live_tx` | 2026-08-30 |
| P0.3 | p0 | uring leftover sweep | done | `cqe_leftover_sequence_never_false_ok` | 2026-08-30 |
| P0.4 | p0 | campanha REAL 3 seeds | done | `l28_real_tcp_removed_campaign_seeds` | 2026-08-30 |
| P1.1 | p1 | planta d=3 + PCT d=3 | done | `planted_chain3_found_by_pct_d3` | 2026-08-30 |
| P2.1 | p2 | pisos nomeados | done | este RFC | 2026-08-30 |
| P2.2 | p2 | close-texts + lint + freeze | done | `residuals.json` | 2026-08-30 |

## Acceptance Criteria

- **Tests**
  - `posix_unsafe_rc_sites_all_gated` cobre ≥ 6 sites e falha com mensagem de linha ao encontrar site sem gate.
  - `capi_len_boundary_sweep_on_live_tx`: nenhuma combinação pânica; todo oversize retorna `MONTAHA_FDB_LIMIT`; null+len>0 retorna erro de argumento; AS-IS (`c_len_admitted_as_is`) admite — dente.
  - `cqe_leftover_sequence_never_false_ok`: 64 tags únicas não nulas; `cqe_act(t_j, want_i) == Discard` para todo j ≠ i; `submit_complete_act(ok|err, false) == WaitMore`; `cqe_res_ok(-5) == false`; dente AS-IS.
  - `l28_real_tcp_removed_campaign_seeds`: 3 seeds, cada uma com `napply=1` e kernels verdes; contabilidade de retry por seed impressa.
  - `planted_chain3_found_by_pct_d3`: d=2 0/256 (estrutural), d=3 ≥1 em 0..16383 (medido 8/16384); `forall_schedules_admitted(3)` false; default `pct_campaign_default_depth() == 2`.
  - `python3 scripts/formal/pedra_formal.py --lint` termina `0 fail`.
- **Telemetry / Analytics:** nenhuma — invariantes fail-closed.
- **Documentation:** este RFC; close-texts dos cinco fortalecidos nomeiam 0156. Nenhuma linha de residual apagada.
- **Screenshots:** backend-only.

## Out of scope

- Extrair `db.rs` (`db_rs_extracted` segue false). Subir o default do PCT acima de 2. Convidado TCG / SSH.
- Apagar ids do `never_floor`: R-cpu, R-rustc, R-verus, R-crc, R-deps, R-extract. Apagar as nove linhas.
- rustfmt de `lib.rs`. Benches / RFC-0149 / 0153 / 0154. Pedra vs Rocks `WriteOptions.sync=true`.
- “Garantia total”, “sem bugs”, seL4, “perfeito”, “acabou”. ∀ traces de TCP. ∀ interleavings de lock do SO. Provar a mídia do userspace. Colisão de CRC impossível.
