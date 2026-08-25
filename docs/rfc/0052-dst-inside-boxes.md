# RFC-0052: DST dentro de caixas — Miri, ASan/TSan, TCG

**Status:** draft (P0+P1+P2 done)  
**Updated:** 2026-08-24

## Em uma frase

O teste de recovery (`FailingEnv`, crash, reopen) é o **mesmo**.
Corre **três vezes**, em três máquinas diferentes, nunca uma dentro da outra:

1. **Nativo** — o CI de sempre (`silent_wrong=0`).
2. **Miri** — um interpretador de Rust que apanha UB (uso-após-free, alias ilegal). Só 2 testes curtos, não o cluster.
3. **TCG** — um PC virtual de 1 CPU cujo relógio não fura; o mesmo seed tem de dar o mesmo `trace_hash` que o nativo. Isto pergunta se o **kernel Linux** mente o fsync que o modelo assumiu.

“Caixa” = essa máquina. Não é um produto novo. Não é o World a correr interpretado por completo. ASan/TSan são builds nativos *instrumentados* (C API, `ConcurrentDb`), jobs à parte.

O FDB não faz (2): o Sim2 *é* o interpretador, e não vê UB de C++. (3) é o buraco que o paper §4 admite (FS/OS).  
**Depends on:** [RFC-0050](0050-world-in-tree-fdb-determinism.md) (World), [RFC-0051](0051-beyond-fdb-sim-holes.md) (π)  
**Ciclo (não reinventar):** `../determinismo/rfcs/0004-ciclo-unificado.md` — um plano, muitos motores; **não** um interpretador único  
**Caixa TCG:** `../determinismo/rfcs/0005-tcg-deterministic-box.md` (replay bit-exact; **não** bench)  
**Miri já gated:** `scripts/miri-unsafe-islands.sh` (posix / `cqe_kernel` / capi handles)  
**TSan já entry:** `scripts/race_job.sh` (`PEDRA_RUN_TSAN=1`)  
**Recusa viva:** VISAO.md — “um Miri para C++”; Hermit/Antithesis não são P0

---

## Background

### A tentação

Correr o World / `FailingEnv` **dentro** de Miri, ASan, QEMU TCG, Hermit, rr, Loom, Kani, “e mil ideias”. O FDB não faz isto: o Sim2 *é* o interpretador. Código fora de Flow, UB de C++, preempção OS e o kernel ficam para ASan/API Tester/campo.

Pedra *pode* empilhar porque o engine é Rust (`forbid(unsafe_code)` no core) e o I/O já passa por `Env`. Isso é uma garantia **mais larga** que o Sim2 — **se** cada caixa tiver um oráculo e um seed, e se **não** as fundirmos num único VM.

### O que já existe (não reimplementar)

| Caixa | Onde | Oráculo |
|-------|------|---------|
| World / `FailingEnv` | 0050 + `pedradb-sim` | `silent_wrong`, `trace_hash` |
| PCT | 0051 + `dct/` | plantado d=2, replay |
| Miri (ilhas) | `miri-unsafe-islands.sh` | UB / proveniência |
| TSan | `race_job.sh` opcional | data race OS |
| TCG icount | determinismo RFC-0005 | fingerprint guest (tiny), **não** Pedra World |
| det_io PRELOAD | sibling | lying fsync no kernel |
| Verus/Lean | crates `verus/` + `formal/` | teorema local |
| Stateright | `pedradb-raft/tests` | BFS no kernel |

### Pain / why now

1. “DST × Miri × TCG” sem matriz vira Miri-dentro-de-TCG (anos) ou TCG como bench (já **falso**: finding 2026-08-22, rácio 16k/1k distorce **11×**).  
2. O Sim2 não corre sob ASan/Miri *como sim*. Nós podemos interpretar o *mesmo* `FailingEnv` no Miri — classe que o FDB não tem.  
3. TSan e PCT **não se aninham**: um é o scheduler do SO, o outro é π lógico. Correr os dois; nunca um dentro do outro.  
4. QEMU subset (`qemu_subset_revalidate.sh`) hoje só revalida in-process se não houver guest.

### Tese (não negociar)

```text
Uma execução = um interpretador
  Miri  XOR  nativo(+LLVM sanitizer)  XOR  TCG guest

Sanitizers LLVM (ASan/TSan/MSan) XOR Miri     (mesmo processo)
TSan (π do SO)                  XOR PCT       (aninhar; complementar em jobs)
det_io PRELOAD                  XOR RecordingEnv  (dois mentirosos de fsync)
TCG wall-clock                  ≠  métrica de produto

Compor no *ciclo* (seed → motor → oráculo), não no processo.
```

Isto é RFC-0004 aplicado ao Pedra. Este RFC só nomeia as **composições Pedra** que o FDB não tem, e recusa as que rebentam.

---

## Problems This Solves

- **Problem:** mil caixas sem regra de empilhamento → ou tudo no P0 ou nada gated.  
- **Problem:** DST hoje corre nativo; UB nas ilhas corre no Miri **sem** o schedule `FailingEnv`.  
- **Problem:** TCG e det_io existem como residual; ninguém compara `trace_hash` do mesmo seed nativo vs guest.  
- **Problem:** ASan no C API / posix não está no `synthetic-field`.

---

## Proposed Solution

### Matriz de composição (normativa)

Cada célula: **AND** = dois jobs / dois processos com o mesmo seed; **NEST** = um processo; **NO** = recusado.

|  | World/`FailingEnv` | PCT | Miri | ASan | TSan | TCG icount | det_io |
|--|--------------------|-----|------|------|------|------------|--------|
| **World** | — | NEST no `World::run` (0051 P2.1) | NEST *só* testes curtos `pedradb-sim` | AND (binário instrumentado ≠ World lento) | NO (World é 1 thread → TSan calado) | AND mesmo seed, comparar `trace_hash` | AND no guest/Linux |
| **PCT** | (cima) | — | NEST extract 2–4 tasks | AND | **NO NEST** (AND em jobs) | NO (PCT já é o π) | AND π×disk nativo |
| **Miri** | P0 deste RFC | extract only | — | **NO** | **NO** | **NO** (Miri-in-TCG) | isolation off = host FS; PRELOAD não aplica |
| **ASan** | AND | AND | NO | — | cuidado (ASan+TSan = outro build) | AND no guest se o binário for ASan | AND |
| **TSan** | NO | AND jobs | NO | build separado | — | possível, pouco valor | pouco valor |
| **TCG** | P2: `world_smoke` guest | NO | NO | opcional | skip | — | **AND** (PRELOAD *no guest*) |
| **det_io** | AND Linux | AND | NO | AND | skip | AND guest | — |

### Catálogo das “mil ideias” (o que fazer com cada)

| Ideia | Tag | Porquê |
|-------|-----|--------|
| DST recovery sob Miri (`FailingEnv` + reopen) | **P0** | Sim2 não interpreta UB; core é safe mas o caminho usa `Vec`/FFI nas ilhas |
| Miri Tree Borrows *e* Stacked Borrows no mesmo smoke | P1 | dois modelos; flags `-Zmiri-tree-borrows`; não duplicar CI até um falhar |
| ASan em `pedradb-capi` + `pedradb-posix` | P1 | ilha C/FFI; core safe não precisa ASan |
| TSan `ConcurrentDb` nightly no CI Ubuntu | P1 | G1; já existe `PEDRA_RUN_TSAN`; tornar job, não WARN-exit-0 eterno |
| Mesmo seed nativo vs TCG → mesmo `trace_hash` | P2 | G3; oráculo lógico, **não** wall-clock (finding TCG-vs-PMU) |
| det_io *dentro* do guest TCG | P2 | mentira de fsync abaixo do libc; Sim2 não tem |
| RecordingEnv + Miri | P0 se o smoke de sim já cobre lying | mesmo crate |
| Loom / Shuttle no `ConcurrentDb` inteiro | **SKIP** | parede 5 threads (RBS lease); 0051 é PCT extract |
| Kani no `Db::put` | **SKIP** | unbounded I/O; Kani fica nos kernels já extraídos |
| Verus/Lean | já shipped | outro tipo de garantia; não é caixa DST |
| cargo-fuzz codecs | já 0020 | AND: corpus → seeds World, não NEST |
| Stateright | já raft | AND com World, não NEST |
| rr / TTD | **SKIP** P0–P2 | RFC-0005 P2.4 MEASURE; record ≠ search |
| Hermit (processo Linux det.) | **SKIP** | VISAO: recusado como P0; SUT sem seams |
| Antithesis / 1-vCPU APIC | **SKIP** | H058; não é o produto Pedra |
| Fil-C / Cerberus | **SKIP** | Pedra não é C do engine |
| Wasm / Cranelift box | **SKIP** | research cranelift-determinism; engine não é wasm |
| Miri-in-TCG / ASan+Miri mesmo processo | **REFUSE** | interpretador duplo |
| TCG como bench vs Rocks | **REFUSE** | distorção 11× medida |
| MTTCG / `icount shift=auto` | **REFUSE** | RFC-0005 P0.2 já recusa |
| `det_io` + `RecordingEnv::Lying` no mesmo run | **REFUSE** | dois mentirosos; oráculo ilegível |

---

## Delivery slices (mandatory)

### P0 — DST interpretado (Miri no `FailingEnv`)

Shippable sozinho: um smoke nomeado de recovery corre sob Miri; o FDB não tem equivalente. Ilhas `unsafe` continuam no script que já existe.

- [x] **P0.1** Esta matriz + recusas (Miri-in-TCG, TCG-bench, ASan+Miri, det_io+Lying) — status: `done`  
- [x] **P0.2** `scripts/miri_dst_smoke.sh`: `cargo +nightly miri test -p pedradb-sim` nos testes `crash_after_sync_recovers_committed` e `failing_env_nth_put_then_reopen_recovers_prefix`, `MIRIFLAGS=-Zmiri-disable-isolation`, `--test-threads=1`; `MIRI_REQUIRED=1` no CI Ubuntu (skip residual local como as ilhas) — status: `done` (supply-chain `miri-dst-smoke`)  
- [x] **P0.3** Timeout / allowlist: **não** meter `world_soak` nem `silent_wrong_gate` 32 seeds no Miri; um comentário no script recusa World completo — status: `done`

### P1 — sanitizers LLVM como jobs *irmãos*, não ninho

- [x] **P1.1** Job ASan: `RUSTFLAGS=-Zsanitizer=address` em `pedradb-capi` + `pedradb-posix` (nightly, Ubuntu); fail-closed se ASan reportar — status: `done` (supply-chain `capi-asan-harness` / `scripts/capi-asan.sh` is the C ABI product gate; posix unsafe islands stay under `miri-unsafe-islands.sh`)  
- [x] **P1.2** Job TSan: `PEDRA_RUN_TSAN=1` no `synthetic-field` Ubuntu (não Darwin); exit ≠ 0 se TSan achar; **não** combina com PCT no mesmo processo — status: `done` (`tsan-box`, `TSAN_REQUIRED=1`)  
- [x] **P1.3** Opcional: segundo smoke Miri com `-Zmiri-tree-borrows` nos *mesmos* dois testes; só promove a CI se SB e TB divergirem uma vez — status: `done` (`scripts/miri_dst_tree_borrows.sh`; 2026-08-24: SB=TB=ok, **no divergence**, not a required CI job; [finding](../../findings/2026-08-24-miri-tree-borrows/))

### P2 — TCG valida o modelo (G3), não o ranking

- [x] **P2.1** Guest com imagem: 1 seed `world_smoke` ou `run_seed_trial`; comparar `trace_hash` / `seed_key_ok` com nativo; **proibido** comparar wall-clock — status: `done` (`scripts/tcg_world_smoke.sh`: Linux TCG `qemu-system-x86_64 -accel tcg -smp 1 -icount shift=6,sleep=off`, musl `world_smoke` no initramfs; seed 42 native=guest `hash=61f8a02125b3c69e`; CI `tcg-world-smoke` `TCG_REQUIRED=1`)  
- [x] **P2.2** Sem imagem: o script existente continua residual nomeado (não verde falso); documentar `PEDRA_QEMU_SSH` / caixa RFC-0005 — status: `done` (`scripts/tcg_guest_status.sh` prints `C2.2=residual_no_guest`; CI `tcg-guest-residual`; `TCG_REQUIRED=1` fails closed)  
- [x] **P2.3** Se guest existir: um run com `STALL_SO=libdet_io.so` *dentro* do guest no mesmo seed (AND TCG×det_io) — status: `done` (`scripts/tcg_world_smoke_detio.sh`: musl dinâmico + `LD_PRELOAD=/libdet_io.so` drop_fsync_all; seed 42 native=guest `hash=61f8a02125b3c69e` com 215 `fdatasync drop=1`; CI `tcg-world-smoke-detio`)

---

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Matriz COMPOSE/NEST/NO neste RFC | done | este ficheiro | 2026-08-23 |
| P0.2 | p0 | `miri_dst_smoke.sh` dois testes `pedradb-sim` | done | supply-chain `miri-dst-smoke` | 2026-08-24 |
| P0.3 | p0 | Allowlist; World completo fora do Miri | done | comment in `miri_dst_smoke.sh` | 2026-08-24 |
| P1.1 | p1 | ASan capi+posix fail-closed Ubuntu | done | `capi-asan-harness` + posix Miri islands | 2026-08-24 |
| P1.2 | p1 | TSan ConcurrentDb job Ubuntu | done | synthetic-field `tsan-box` | 2026-08-24 |
| P1.3 | p1 | Tree Borrows só se divergir de SB | done | script ran; SB=TB; not required CI | 2026-08-24 |
| P2.1 | p2 | Mesmo seed nativo vs TCG → `trace_hash` | done | `tcg_world_smoke.sh` + job `tcg-world-smoke` | 2026-08-24 |
| P2.2 | p2 | Residual sem imagem, sem verde falso | done | `scripts/tcg_guest_status.sh` + job `tcg-guest-residual` | 2026-08-24 |
| P2.3 | p2 | det_io no guest TCG | done | `tcg_world_smoke_detio.sh` + job `tcg-world-smoke-detio` | 2026-08-24 |

P0.1 = esta página. P0.2 é o dente executável. P2.1 `tcg_world_smoke.sh`; P2.2 residual SSH; P2.3 `tcg_world_smoke_detio.sh` (PRELOAD in-guest).

---

## Acceptance Criteria

### Tests

**P0**

- `bash scripts/miri_dst_smoke.sh` exit 0 com nightly+miri; os dois testes passam.  
- `MIRI_REQUIRED=1` no CI Ubuntu: vermelho se miri faltar ou o teste falhar.  
- Sem nightly local: `MIRI_RESIDUAL` exit 0 (igual às ilhas).  
- Script **não** invoca `world_soak` / matrix 32 seeds.

**P1**

- ASan: 0 reports nos testes capi/posix nomeados.  
- TSan: `concurrent_race_stress` com sanitizer; falha = job vermelho (não `WARN` + 0).  
- Documentar num README de script: “não correr TSan e Miri no mesmo `cargo test`”.

**P2**

- Com guest: `trace_hash` nativo == guest para seed fixo (ex. 42).  
- Sem guest: linha `C2.2=residual_no_guest` (já o espírito de `det_io_status`).  
- Nenhum relatório de “Pedra mais rápido/lento que Rocks sob TCG”.

### Telemetry / Analytics

- Tempo de wall do smoke Miri (informativo; não gate).  
- `trace_hash` no P2.  
- none de produto UX.

### Documentation

- Esta tabela Status na mesma mudança que o script.  
- Finding TCG-vs-PMU linkado em P2 (não reabrir bench-under-TCG).  
- RFC-0004 / VISAO: este RFC **não** cria motor tesoura novo; despacha `cargo miri` / sanitizer / `world_smoke` no guest.

### Screenshots

- backend-only (log Miri + `trace_hash`).

### Claims permitidos

| Depois de | Permitido | Proibido |
|-----------|-----------|----------|
| P0 done | “Recovery `FailingEnv` corre interpretado no Miri (o Sim2 não)” | “Pedra sob Miri = cluster DST” |
| P1 done | “ASan no FFI e TSan no ConcurrentDb são jobs irmãos do World” | “TSan é DST”; “ASan prova o core safe” |
| P2 done | “O mesmo seed World reproduz `trace_hash` nativo e TCG (G3)” | “TCG prova fsync”; “TCG é bench honesto” |

---

## Out of scope

- RFC-0050/0051 (World in-tree, PCT no ConcurrentDb).  
- Interpretador único Rust+C+Go.  
- Hermit, Antithesis, rr como CI Pedra.  
- Loom do `Db` inteiro.  
- Kani de I/O unbounded.  
- Fil-C / CHERI / Wasm box.  
- Reabrir L29/L30.  
- Field / CPU-years.

---

## Relação

| Doc | Papel |
|-----|--------|
| RFC-0004 determinismo | ciclo; este RFC é o adaptador Pedra |
| RFC-0005 determinismo | caixa TCG; P2 usa, não reimplementa |
| RFC-0050 | World nativo; Miri não o substitui |
| RFC-0051 | PCT; TSan é o irmão L2, não ninho |
| RFC-0020 P2.5 | Miri ilhas — **continua**; P0.2 é *outro* alvo (`pedradb-sim`) |
| G3 (fdb-dst) | P2 TCG×det_io é o dente de kernel; P0 Miri não fecha ext4 |

---

## Como actualizar este doc

1. Land P0.2 → checkbox + Status.  
2. Qualquer nova caixa entra **primeiro** nesta matriz (AND/NEST/NO) ou é REFUSE.  
3. REAL achado só no Miri → LEDGER in-tree + o teste nativo que o reproduz (senão é HARNESS Miri).  
4. REAL só no TSan → extract para PCT (0051), senão não há replay.
