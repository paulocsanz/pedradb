# RFC: o destino de dados que corre *é* o termo (resto da conta seL4)

**Status:** done
**Updated:** 2026-09-06
**Parents:** [0171](0171-pagar-o-preco-sel4.md), [0172](0172-rustc-translation-validation-one-target.md), [0061](0061-residuals-sel4-ironfleet.md)
**Dono:** o autor pediu o RFC para “sermos seL4”. Este documento **não** autoriza a frase “somos seL4”. Autoriza o resto da conta que eles pagaram no *quê* corre.

**Frase deste RFC:** cada `if` de destino de dados que o binário executa vive no ficheiro que o `rustc` liga e que o prover vê. `db.rs` é trampoline `Env`. TV no pin 0172 cobre os objectos desses artefactos, um fn de cada vez. `never_floor` não mexe.

**Ainda recusado (seL4 também não):** “sem bugs”, CPU, rustc *em geral*, Z3, colisão CRC, mídia `fsync`, `∀` interleavings. `R-cpu` `R-rustc` `R-verus` `R-crc` `R-deps` **ficam** `never`. `glue.db_rs_extracted` **fica** `false`.

## Background

- seL4: o C que corre *é* o termo (~10 kLOC escritos para a prova) + TV do binário num ARM. Não provaram o gcc, o CPU, nem “o computador”.
- RFC-0171 pagou isso **num** artefacto (`write_admission_kernel.rs`) e no caminho put-Ok + recover. RFC-0172 pagou o passo ARM **num** fn (`write_admission_idle` IR, 8/8) no pin aarch64-apple-darwin / rustc 1.97.1.
- O resto do engine ainda tem gémeo ao lado (`verus/foo.rs` ≠ o `.rs` que liga) e `db.rs` ainda tem destino de dados fora desse artefacto (get/scan/compact/TX). Isso **não** é a classe de garantia seL4. É a classe 0170 (decisões extraídas).
- “Sermos seL4” como booleano é mentira mesmo no fim deste RFC: não somos um microkernel Isabelle. O que este RFC fecha é o eixo que o 0061 chamou *o que foi escrito para o prover* — no Pedra: o Rust de destino de dados que **corre**.

## Problems This Solves

- **Problem:** 0171+0172 pagam um ficheiro e um fn. A pergunta “e agora, somos seL4?” continua a ser respondida com recusa ou com slogan.
- **Problem:** a maior parte dos pares `data_fate` ainda tem `twin` ≠ `kernel`.
- **Problem:** sem um freeze da fracção `single_artifact`, o resto da conta é invisível.

## Proposed Solution

Três eixos, na ordem seL4:

1. **Todos os `data_fate` são artefacto único.** `twin` path = `kernel` path; Verus no ficheiro que o crate liga (padrão 0171: `macro_rules!` + `cfg(verus_keep_ghost)`). Um par por turno depois do P0.
2. **`db.rs` = trampoline.** Cada `if` de destino de dados (get/scan/compact/flush/TX) chama um kernel do artefacto único. I/O `Env` fica. Não se dumpa o ficheiro.
3. **TV no pin 0172.** Depois de idle: `write_admit`, depois um fn por artefacto novo. Continua a não ser CompCert. `R-rustc` never.

## Delivery slices (mandatory)

### P0 — o mapa existe e o segundo artefacto é um só ficheiro

- [x] **P0.1** Este RFC: resto da conta nomeado (todos os `data_fate` = artefacto único; `db.rs` trampoline; TV no pin 0172) — status: `done`
- [x] **P0.2** Freeze da fracção: `--lint` publica `single_artifact` / `data_fate` counts; teste nomeado falha se o freeze no `residuals.json` (ou catalog stamp) não bate no live — status: `done`
- [x] **P0.3** Segundo ficheiro de produção é o termo (candidato: `flush_kernel.rs`; hoje o twin é `verus/flush_decision.rs`). Verus no path que liga; `single_artifact: true`; lint recusa twin ≠ kernel nesse par — status: `done`

### P1 — get/scan no mesmo critério do put-Ok

- [x] **P1.1** `get_at` / lookup: zero `if` de destino de dados em `db.rs` (só chamadas a kernels artefacto-único; `visible_at` / `probe_order` candidatos) — status: `done`
- [x] **P1.2** Planta nomeada (grep de `if` nesses fns = vazio de predicado cru) — status: `done`
- [x] **P1.3** Aeneas SOURCE do artefacto do P0.3 (produção = extract, sha256) — status: `done`

### P2 — o passo ARM no resto do pin

- [x] **P2.1** TV de `write_admit` no mesmo pin 0172 (IR ⊭ spec falha o script) — status: `done`
- [x] **P2.2** `handler_loc` estritamente abaixo do freeze 0171; inventário do trampoline get/scan — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | mapa do resto da conta | done | this RFC | 2026-09-06 |
| P0.2 | p0 | freeze fracção single_artifact | done | test_single_artifact.py | 2026-09-06 |
| P0.3 | p0 | flush_kernel artefacto único | done | verus_flush_decision.sh | 2026-09-06 |
| P1.1 | p1 | get/lookup sem if cru em db.rs | done | lookup_kernel.rs | 2026-09-06 |
| P1.2 | p1 | planta nomeada get path | done | get_at_and_lookup_path_data_fate_ifs_call_kernels | 2026-09-06 |
| P1.3 | p1 | Aeneas do 2º artefacto | done | aeneas_flush.sh / Flush.lean | 2026-09-06 |
| P2.1 | p2 | TV write_admit no pin 0172 | done | tv_write_admission.sh | 2026-09-06 |
| P2.2 | p2 | handler_loc a descer | done | findings/2026-09-06-rfc0174-trampoline/ | 2026-09-06 |

## Acceptance Criteria

- **Tests**
  - P0.2: freeze live = stamp; mutante com `single_artifact` count errado falha por id.
  - P0.3: `verus <flush_kernel.rs>` (ou o path que o crate liga) verde; lint falha se `twin` ≠ `kernel` nesse par.
  - P1.2: planta nomeada no get path.
  - P2.1: `tv_write_admission.sh` (ou sucessor) TVs `write_admit`; pin 0172 inalterado.
- **Telemetry / Analytics:** none — prova; o sinal é exit code + stamp.
- **Documentation:** este RFC; linha `docs/status.md`; 0171 aponta para aqui como resto da conta.
- **Screenshots:** backend-only.

## Out of scope

- Dump de `db.rs` (`glue.db_rs_extracted` fica `false`).
- Tirar `R-cpu` / `R-rustc` / `R-verus` / `R-crc` / `R-deps` do `never_floor`.
- CompCert / prova do LLVM / todos os alvos.
- Frase “somos seL4” / “sem bugs” mesmo com P0–P2 `done`. A frase permitida é: destino de dados do engine que corre ⊨ spec, relativo ao TCB A.
- Benches / crates.io / Rocks parity.
- `∀` interleavings de `ConcurrentDb`.
