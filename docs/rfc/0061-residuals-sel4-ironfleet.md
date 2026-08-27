# RFC-0061: Inventário único dos residuais — Pedra vs seL4 / IronFleet

**Status:** done (P0–P2)
**Updated:** 2026-08-25
**Parents:** [0053](0053-ironfleet-years.md) (Y1–Y3), [0056](0056-one-hundred-percent-delivery.md) (100% relativo ao TCB), [0057](0057-maximum-intensity-parallel-dst-boxes-formal.md), [0052](0052-dst-inside-boxes.md), [0060](0060-field-and-hardware-residuals.md), [coverage-map](../formal/coverage-map.md)
**Fonte máquina:** [`scripts/formal/residuals.json`](../../scripts/formal/residuals.json) (freeze no `pedra_formal.py --ci`)

**Resposta à pergunta deste RFC:** **não**. Pedra não é tão robusto quanto seL4, nem quanto o IronRSL. Está na **mesma classe de claim** (prova relativa a um TCB escrito, residual publicado, nunca “sem bugs”). Não está na **mesma classe de garantia** (o *quê* foi provado, em que linguagem, com que rácio prova:impl).

## Background

- O checklist de “100% relativo ao TCB” fechou (RFC-0056). A frase que o CI e os docs permitem é: kernel K ⊨ spec S, relativo a axiomas A.
- A frase que o mesmo programa **proíbe**: “não há bugs no Pedra”, “Pedra verificado”, “fsync provado”, “tão robusto quanto seL4”.
- Os residuais viviam espalhados: coverage-map, relatório 0056, RFC-0060, parked de remesura (0031–0044), Evacuate (0038), TLS (0021), extração recusada (L46). Sem um inventário único, “100% relativo ao TCB” arredonda para “tão robusto quanto seL4”.
- seL4: ~10 kLOC C, ~200 kLOC Isabelle; hardware, boot e a spec continuam fora. IronRSL: 5 114 impl Dafny + 39 253 prova (3.6:1); o **loop principal** ficou no TCB. Pedra: ~7.7 kLOC de kernels de decisão + twins 0.65:1, dentro de ~40 kLOC de handlers / ~76 kLOC de `src` nas crates formalizadas. Produção é Rust; o prover vê o kernel extraído, não o binário inteiro.

## Problems This Solves

- **Problem:** a pergunta “estamos tão robustos quanto seL4/IronFleet?” não tem uma página que recuse o arredondamento.
- **Problem:** residual sem dono parece inexistente; residual parked parece “falha de prova”.
- **Problem:** o freeze do TCB recusa kernel novo sem par, mas **não** recusa um residual novo sem linha.

## Proposed Solution

1. **Uma tabela** (este RFC + `residuals.json`): residual → classe (`never` | `continuous` | `parked` | `open`) → RFC dono → mecanismo vivo → o que *fecharia* (ou por que nunca fecha).
2. **Comparação honesta** seL4 / IronRSL / Pedra na mesma página — três eixos, nunca um número só.
3. **Freeze:** `pedra_formal.py --ci` recusa catálogo malformado, id duplicado, classe inválida, ou dono sem ficheiro `docs/rfc/`.
4. **Não** provar o Linux, não reescrever em Isabelle, não “fechar” o item 12. Atacar só o que muda o mapa sem mudar o TCB.

### Comparação (três eixos, não um score)

| Eixo | seL4 | IronRSL (IronFleet) | Pedra |
|---|---|---|---|
| O que foi escrito para o prover | o kernel | o Paxos+KV em Dafny | **não** — kernels extraídos do Rust de produção |
| O que a prova cobre | implementação do microkernel vs spec | protocolo + KV vs spec | **decisões** de destino de dados (voto, AE, commit, WAL recover, flush, compact, group-commit, …) |
| Rácio prova:impl (ordem de grandeza) | ~20:1 Isabelle:C | 3.6:1 | twins ~0.65:1 (a prova cobre a decisão, não o I/O) |
| Loop / glue | kernel provado; userland fora | loop principal **no TCB** | loop TCP modelado em Stateright; glue ~40–69 kLOC **no TCB à vista** |
| Concorrência | modelo de interrupção | 1 thread | kernel de grupo + PCT d=2 (fatia); `∀` interleavings **fora** |
| Disco / `fsync` | N/A (não é KV) | axiomas IOSystem | `FailingEnv` + smoke TCG; **não** prova de mídia |
| Hardware | fora | fora | fora (RFC-0060 = CRC+scrub, não ECC) |
| Frase permitida | “kernel functionally correct wrt spec” | “Paxos+KV correct wrt spec” | “kernels de decisão correct wrt spec; glue TCB; DST ataca o resto” |
| Frase proibida | “o computador não tem bugs” | “não há bugs no datacenter” | “tão robusto quanto seL4” / “sem bugs” |

**Leitura:** seL4 e IronFleet pagaram o preço de **escrever o sistema no prover** (ou um kernel pequeno o suficiente). Pedra pagou o preço inverso: produção em Rust, kernels extraídos, DST no glue. Isso é mais DST que o Sim2 do FDB em alguns eixos (Miri no core `forbid(unsafe_code)`, World in-tree) e **menos prova de implementação** que seL4. Não é um empate.

### Classes

| Classe | Significa | Fecha? |
|---|---|---|
| `never` | axioma, outro projecto (CompCert, seL4, Ironclad), ou recusa explícita | não — mudar a classe é uma decisão do dono, não um fix |
| `continuous` | item 12: ataca-se para sempre (campanha, não teorema) | não “fecha”; o mecanismo vivo tem de continuar verde |
| `parked` | dono decidiu não fazer agora (Evacuate, remesura quieta, pick C, TLS default, iff 0055) | só o dono |
| `open` | implementável neste RFC ou no dono apontado, sem mudar o TCB | sim, com teste nomeado |

### Never-list (P2.1 — freeze)

Estes ids são o piso `never`. Apagar um deles do json sem o tirar **também**
deste RFC (e de `never_floor`) faz o `--ci` falhar.

`R-cpu` · `R-rustc` · `R-verus` · `R-crc` · `R-deps` · `R-extract`

## Delivery slices (mandatory)

### P0 — o mapa existe e o CI recusa-o podre (útil sozinho)

- [x] **P0.1** Este RFC: comparação seL4/IronRSL/Pedra + tabela humana dos residuais — status: `done`
- [x] **P0.2** `scripts/formal/residuals.json` + `check_residuals` no `pedra_formal.py --ci` (id único, classe ∈ {never,continuous,parked,open}, dono = RFC cujo ficheiro existe) — status: `done`
- [x] **P0.3** coverage-map + open-items apontam para este RFC como inventário único — status: `done`

### P1 — atacar o que dá para atacar sem virar seL4

- [x] **P1.1** Cada ilha `unsafe` (`pedradb-posix`, `pedradb-io-uring`, `pedradb-capi`) tem linha no json com o script Miri/ASan **vivo** e o buraco nomeado (SAFETY.md ≠ prova ∀) — status: `done`
  (`R-unsafe-posix` → `scripts/miri-unsafe-islands.sh`;
  `R-unsafe-uring` → `scripts/miri-unsafe-islands.sh`;
  `R-unsafe-capi` → `scripts/capi-asan.sh`; freeze exige os paths)
- [x] **P1.2** Guest TCG: enquanto não houver `PEDRA_QEMU_SSH`, o json fica `continuous` com mecanismo `tcg_guest_status.sh` (`C2.2=residual_no_guest`); `TCG_REQUIRED=1` continua fail-closed — status: `done`
  (`R-tcg-guest`; freeze lê o script; não se inventou guest)
- [x] **P1.3** Glue: publicar no json o número kernel/handler LOC e o freeze já existente (RFC-0056 P2.5); **não** extrair `db.rs` — status: `done`
  (`glue.kernel_files` / `kernel_loc` / `handler_loc` = live count; `db_rs_extracted: false`)

### P2 — lista never congelada + residuais de produto que não são prova

- [x] **P2.1** A classe `never` só muda com PR que edita **este** RFC e o json (comentário no check se um id `never` desaparecer) — status: `done`
  (`never_floor` + grep RFC-0061; teste `test_residuals_freeze.py`)
- [x] **P2.2** TLS default / encrypt-at-rest: dono continua [0021](0021-security-tls-baseline.md) (lab shipped, não GA) — status: `done`
  (`R-tls` parked; **não** se ligou TLS-by-default)
- [x] **P2.3** Joint consensus de membership Montanha: dono passou a [0063](0063-fdb-reliability-close-the-system-gap.md) — status: `done`
  (`R-joint` **open**; P0 log-carried `MembershipJoint` shipped 2026-08-26; election-time joint = 0063 P1.1)

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + comparação seL4/IronFleet | done | este doc | 2026-08-25 |
| P0.2 | p0 | residuals.json + freeze CI | done | `check_residuals` no `--ci` | 2026-08-25 |
| P0.3 | p0 | coverage-map + open-items | done | ponteiro 0061 | 2026-08-25 |
| P1.1 | p1 | ilhas unsafe nomeadas no json | done | R-unsafe-posix/uring/capi + script paths | 2026-08-25 |
| P1.2 | p1 | TCG guest residual honesto | done | R-tcg-guest + tcg_guest_status.sh | 2026-08-25 |
| P1.3 | p1 | glue LOC no json + freeze | done | glue.kernel_files/loc live; no db.rs | 2026-08-25 |
| P2.1 | p2 | never-list só muda aqui | done | never_floor + RFC grep + drop test | 2026-08-25 |
| P2.2 | p2 | TLS default (produto) | done | R-tls parked; RFC-0021 owner | 2026-08-25 |
| P2.3 | p2 | joint consensus membership | done | R-joint open; RFC-0063 P0 | 2026-08-26 |

## Acceptance Criteria

- **Tests**
  - P0.2: `python3 scripts/formal/pedra_formal.py --lint` inclui `ok residuals freeze: N rows`; json malformado / classe inválida / dono sem RFC ⇒ FAIL.
  - P1.1: freeze exige `script`+`safety` existentes para posix/io-uring/capi.
  - P1.2: freeze exige `scripts/tcg_guest_status.sh` e `TCG_REQUIRED` fail-closed no script.
  - P1.3: freeze exige `glue.kernel_files`/`kernel_loc`/`handler_loc` = live; `db_rs_extracted=false`.
  - P2.1: `scripts/formal/test_residuals_freeze.py` — dropar um id `never` do catálogo sem o piso falha nomeando o id.
- **Telemetry / Analytics**
  - none — inventário de engenharia, não métrica de produto.
- **Documentation**
  - este RFC; `residuals.json`; linha no coverage-map e open-items.
- **Screenshots**
  - backend-only.

## Out of scope (non-goals)

- Tornar Pedra seL4 (reescrever o engine em C+Isabelle, provar o binário).
- Tornar Pedra IronRSL (reescrever em Dafny; extração total recusada, L46).
- Provar Linux, rustc, CRC, ECC, ou o ring io_uring.
- `∀` interleavings do `ConcurrentDb` (escolha 0056 P2.1: kernel de grupo, não π/VerusSync).
- Remesura quieta vs Rocks `sync=false` (parked nos RFC 0031–0044; não é residual de *prova*).
- Evacuate / pick C 0027–0028 / iff 0055 — parked do dono, listados aqui como `parked`.
- Claim de CPU-hours vs FDB Simulation.
