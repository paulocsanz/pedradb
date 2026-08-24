# Entrega do “100% relativo ao TCB” — relatório final (RFC-0056)

**Updated:** 2026-08-23
**Programa:** [RFC-0056](../rfc/0056-one-hundred-percent-delivery.md) · **Contrato:** [one-hundred-percent.md](one-hundred-percent.md)
**Frase canónica:** kernel K ⊨ spec S, relativo a axiomas A — nunca “não há bugs no Pedra”.

---

## 1. O estado dos itens 1–12

| # | Item | Estado | Evidência |
|---|------|--------|-----------|
| 1 | Todo `if` de destino de dados é `fn` puro que produção chama | **verde** (menos o caminho `ConcurrentDb`/group-commit — gate do item 7) | 35 kernels de decisão (freeze-enumerated); flush, compact, MANIFEST, vlog GC, 2PC glue saíram do inline (P0.1–P0.3, P1.4); lint exige o `entry` chamado em cada caller |
| 2 | Cada kernel tem prova ∀ (Verus) | **verde** | 44 pares catalogados, 40 twins; rácio twin:kernel ≈ 0.65:1 (tabela §3); zero `sorry`; drift twin⊇kernel no `--ci` |
| 3 | 2ª máquina (Aeneas→Lean) no extract | **verde** | 8 extracts com drift-stamp sha256 (vote, isolated, bloom, AE, commit, reopen, apply, wal-recover); `vote_decision_iff`; teoremas AS-IS nos 3 kernels P1.2 |
| 4 | Caller refina (grant ⇒ persist Ok, ACK ⇒ commit) | **verde** | F15/F11/F16 kernels + twins; refinamento verificado no loop TCP (P1.3: 8/8); dicionário put→…→get (P1.1, `8 verified`) |
| 5 | Composição: spec dicionário + crash | **verde** | `dictionary_link.rs` encadeia put→WAL→recover→reopen→get com hipóteses nomeadas A1–A4; DST e2e acked-prefix-survives verde no `--ci` |
| 6 | Host loop é máquina de estados | **verde** | `compose_model.rs` (vote∧AE∧commit) + `tcp_node_model.rs` (node inteiro, kernels de produção chamados no passo; 8/8) |
| 7 | Concorrência: redução ou lógica concorrente | **ABERTO (gate)** | `ConcurrentDb` real; PCT não aterrou (RFC-0051 P0); P2.1 gravado como `todo (gated)` — **não** claim de concorrência verificada |
| 8 | Liveness (eleição eventual) | **verde relativo aos axiomas** | P2.2: `Property::eventually` no modelo do node inteiro; bounded liveness sob ES-1/ES-2/ES-3 (§4); refutado sem axiomas — nunca teorema |
| 9 | I/O é transição (crash = reset + torn) | **verde** | `FailingEnv`/`RecordingEnv` por todo o path; P2.3: zero `Db<StdEnv>` hardcode em API de biblioteca (§5); journal+index generalizadas com teste `FailingEnv` e2e |
| 10 | Glue não-kernel é zero **ou** TCB à vista | **verde à vista; zero não** | §3: 7.723 LOC kernel vs 39.793 LOC de ficheiros handler; freeze do TCB (§6) recusa crescimento silencioso — o resto é explícito, não escondido |
| 11 | Twin drift = CI vermelho | **verde** | lint (entry+handlers), clones (tokens idênticos), SOURCE sha256 stamps, freeze; testes negativos em cada wave (P0.4, P1.2, P1.5, §6) |
| 12 | Axiomas atacados para sempre | **contínuo (por definição)** | World sibling (RFC-0050), Miri/TCG (RFC-0052), axiomas ES (§4) agora declarados no TCB; item que nunca “fecha” |

**Leitura honesta:** itens 1–6, 8, 9, 11 verdes; 10 verde no braço “TCB à vista” (zero-glue
continua a trajectória); **7 aberto por gate** (RFC-0051 PCT; π-redução/VerusSync escolhida
quando abrir); 12 contínuo. Isso é o máximo que seL4/IronFleet chamam de sistema verificado —
relativo ao TCB escrito, nunca “não há bugs”.

---

## 2. Números de fechamento

| Métrica | Valor |
|---|---|
| Pares kernel↔twin no catalog | 44 (18 `data_fate`) |
| Kernels de decisão em produção | 35 (34 registados + 1 allowlist cqe_kernel) |
| Twins Verus | 40 ficheiros |
| Extracts Aeneas→Lean com drift-stamp | 8 |
| Modelos Stateright no `--ci` | 32 (incl. `compose_model`, `tcp_node_model`) |
| `pedra_formal.py --lint` | 68 ok, 0 gap, 0 fail |
| `pedra_formal.py --ci` | **212 ok, 0 gap, 0 fail** (bateria final; log no scratch da sessão) |
| Verus bateria final (×2 cada) | manifest 9 · flush 10 · compact 10 · dictionary 8 · reopen 6 · apply 3 · vlog 14 · tx_glue 7 — todos `0 errors` nas duas corridas |
| Verificados novos por wave | P0: 29 (manifest 9 + flush 10 + compact 10) · P1.1: 8 · P1.4: 21 (vlog 14 + tx_glue 7) |
| Teoremas Lean sobre extracts | 17 novos em P1.2 (reopen 4, apply 5, wal-recover 8) + vote/iso/bloom/ae/commit prévios |

## 3. P2.4 — tabela LOC glue vs kernel por crate

Medição (reprodutível): kernels = ficheiros `kernel` do catalog ∪ lados dos clones;
twins = ficheiros `twin`; handler-files = ficheiros `callers` (onde as decisões vivem).
LOC = linhas de ficheiro (incl. docs/testes in-file).

| Crate | Kernel LOC | Twin LOC | Handler-file LOC |
|---|---:|---:|---:|
| montanha-fdb-recipes | 347 | 140 | 1.278 |
| pedradb-core | 3.773 | 2.506 | 18.977 |
| pedradb-dcs | 234 | 160 | 642 |
| pedradb-fold | 149 | 98 | 612 |
| pedradb-http | 916 | 364 | 2.134 |
| pedradb-journal | 78 | 73 | 269 |
| pedradb-raft | 923 | 635 | 2.837 |
| pedradb-replicate | 207 | 202 | 647 |
| pedradb-store | 1.021 | 772 | 12.032 |
| pedradb-stream | 75 | 72 | 365 |
| **Total** | **7.723** | **5.022** | **39.793** |

**Trajecto:** P0 report ≈ 703 twin LOC nos 3 kernels engine (0.84:1); P1 somou 339 kernel /
688 twin; fechamos em **twin:kernel ≈ 0.65:1** global (IronRSL ≈ 3.6:1 — nosso rácio é menor
porque o twin cobre a decisão, não o I/O). O alvo “todo caminho de destino de dados é
abre-fd→kernel→persiste” está garantido **por handler** nos 18 pares `data_fate` (lint
exige o handler a chamar o kernel); o que resta nos handler-files é I/O, codificação e
máquinas de leitura — não decisões de destino (o freeze recusa novas em silêncio).

## 4. P2.2 — liveness e os axiomas ES

`tcp_node_model.rs`: `Evt-apply-quiescence` e `Evt-election` como `Property::eventually`
em comportamentos limitados a `MAX_STEPS = ES_BOUND + 2` (o BFS do Stateright avalia
eventualidade nos estados terminais de modelos path-acyclic — o bound é o que torna o
check sadio e não-vacuo). Valem só sob **ES-1** (adversário finito: crash-restarts
finitos, sem partição infinita), **ES-2** (drain interno até à quiescência), **ES-3**
(candidato vivo em retry) — nomeados no modelo e no TCB
([one-hundred-percent.md §1](one-hundred-percent.md)). Dentes: sem axiomas **ambas**
refutadas; sem ES-3 só a eleição cai (cada axioma é load-bearing); mutante de loop
`broken_drain` é apanhado mesmo com todos os axiomas. 8/8 testes.

## 5. P2.3 — sweep `StdEnv` (classificação final)

`grep -rn "StdEnv" crates/*/src` → 0 hardcodes `Db<StdEnv>` em API de biblioteca.
Hits restantes, por classe:

1. **Defaults de type-param** — `Db<E: Env = StdEnv>`, `DBIterator`, `Snapshot` (sugar; o
   parâmetro injectável existe).
2. **Construtores de conveniência** — `Db::open`, `impl Db<StdEnv>`, `impl
   ConcurrentDb<StdEnv>`, persist path-only APIs: cada um delega à variante injectável
   (`open_with_env`, `*_on`) — o seam é a variante, o default é escolha do consumidor.
3. **Módulos `#[cfg(test)]`** — history, lock, manifest, cache, corrupt, change_feed,
   vlog, dcs, sim.
4. **Wrappers Env by-design** — io-uring `Posix(StdEnv)` fallback dentro do `UringEnv`;
   `DetHost` host-side default.
5. **Leaf consumers** — rocksdb-compat passa o default **à API injectável**
   (`open_cf_with_env`); bins/CLIs escolhem o env.

Ilhas corrigidas nesta wave: `pedradb-journal` (`catch_up/peek/append/changes_after`) e
`pedradb-index` (`put_row_with_indexes/row_fully_indexed/row_half_indexed`) — agora
`<E: Env>`, com teste `FailingEnv` end-to-end em cada crate provando que o oráculo de
falhas da DST percorre o path inteiro.

## 6. P2.5 — TCB congelado no CI

`pedra_formal.py` ganhou `check_tcb_freeze` (corre no `--lint` e no `--ci`):

- Todo ficheiro `crates/*/src/**/*_kernel.rs` tem de ser **par catalog** (kernel+twin+script),
  **clone catalog** (mirror drift-checked) ou **allowlist explícita** — hoje só
  `cqe_kernel.rs` (io_uring: propriedades in-file; twin bloqueado num modelo de ring —
  open item).
- Todo par `data_fate` tem de ter twin + script Verus existentes.
- Entrada de allowlist stale (ficheiro desapareceu) também é vermelho.

**Testes negativos (2026-08-23):** (a) kernel sintético
`zz_freeze_probe_kernel.rs` sem registo → `FAIL tcb freeze: … neither a catalog pair, a
catalog clone, nor allowlisted`, `1 fail`; (b) par `vlog_recover` sem script Verus →
`FAIL tcb freeze: data_fate pair vlog_recover lacks verus script`, `1 fail`. Restaurado →
68 ok / 0 fail. O TCB não cresce em silêncio.

## 7. O que este relatório **não** claima

- Concorrência do `ConcurrentDb` (item 7 / P2.1: gate RFC-0051; π-redução ou VerusSync
  quando o gate abrir).
- Liveness não-axiomática: ES-1/2/3 são axiomas declarados, atacados para sempre (item 12).
- “Zero glue”: 39.793 LOC de handler-files continuam TCB — agora visíveis e congelados,
  não provados.
- θ do blob-GC é política, não teorema; `cqe_kernel` sem twin (allowlist).

## Relation

| Doc | Papel |
|---|---|
| [one-hundred-percent.md](one-hundred-percent.md) | o contrato (itens 1–12) + TCB + axiomas ES |
| [p0-engine-report.md](p0-engine-report.md) · [p1-composition-report.md](p1-composition-report.md) | waves P0/P1 |
| [y1–y3 reports](.) | RFC-0053 (método) |
| [RFC-0051](../rfc/0051-beyond-fdb-sim-holes.md) | gate do item 7 |
