# RFC: 0228 — CRC32C: a classe de detecção provada em Lean; o residual re-escopado de "sem colisões" para axioma de canal

**Status:** done (P0–P1; P2 aberto)
**Updated:** 2026-09-15
**ID:** 0228
**Parents:** catálogo de residuals `R-crc` (classe `never`,
[0060](0060-field-and-hardware-residuals.md) owner; [0061](0061-residuals-sel4-ironfleet.md) classe de
claim, não de garantia),
[0187](0187-teorema-experimento-tcb.md) (teorema ≠ experimento ≠ TCB: um
teorema de matemática pura não vira garantia de produto sozinho),
[0227](0227-prova-de-produto-put-get-recover-replica.md) (TCB intacto;
frases relativas, nunca absolutas)
**Peer:** intocado — este RFC não toca parity nem performance.

> Pergunta que originou (2026-09-15): "dá pra provar o CRC no sentido da
> verificação?" Resposta em três partes: (a) a frase global
> "CRC32C não tem colisões" é **falsa por contagem** e nunca será teorema;
> (b) as **classes de detecção** — burst ≤ 32 bits, peso ímpar, 2 bits com
> span < 2³¹−1 — são teorema e agora estão provadas em Lean, zero `sorry`
> (`formal/aeneas/lean/CrcClass.lean`); (c) o elo com o mundo físico
> (a corrupção de NAND cai dentro da classe detectada) é axioma **para
> sempre** — é o `R-crc`/R-hardware, re-escopado de "sem colisões" para o
> enunciado quantificado honesto.

**Frase permitida** (relativa à classe, nunca global): *todo erro de burst
de comprimento ≤ 32 bits, todo erro de peso de Hamming ímpar, e todo erro
de 2 bits com span < 2³¹−1 é detectado pelo CRC32C — provado em Lean,
zero `sorry`; o que resta é o axioma de canal.*

**Frases recusadas:** "CRC32C não colide", "não existe corrupção
silenciosa", "o CRC está provado", "detecção total".

## Background

- O CRC32C é o único detector de corrupção do engine (WAL, vlog, MANIFEST,
  changelog, bloom sidecar, history segments, checkpoint). O catálogo
  (`scripts/formal/residuals.json`) carrega o `R-crc` como classe `never`
  desde 0060/0061: igualdade de u32 (`crc_match_ok`) não é prova de
  ausência de colisão, e os testes Rust
  (`crc_collision_axiom_remains`, `wal_open_crc_collision_axiom_remains`,
  `checkpoint_crc_collision_axiom_remains`, …) pinam que o axioma
  continua declarado.
- Contagem (por isso "nunca"): mensagens de k bits mapeiam em 2³² CRCs;
  para k > 32 colisões **existem por princípio da casa dos pombos**. Um
  teorema global de não-colisão é impossível; só classes são prováveis.
- Matemática clássica (Rabin, 1954 / Castagnoli et al. 1993, conhecida de
  projeto): sobre GF(2)[X], um erro E passa despercebido ⟺ G ∣ E com
  G = 0x11EDC6F41 = (X+1)·P31, P31 = 0xF5B4253F. Daí:
  - **burst ≤ 32**: G ∣ E com E = X^a·B, B ≠ 0, exige deg B ≥ 32
    (Rabin; aqui via `natDegree G = 32` + cancelamento de X pelo termo
    constante 1).
  - **peso ímpar**: (X+1) ∣ G ∣ E ⟹ E(1) = 0 ⟹ número par de termos.
  - **2 bits**: E = X^a + X^b = X^a(1 + X^(b−a)); P31 ∤ 1 + X^d para
    0 < d < 2³¹−1 porque a ordem de X em GF(2)[X]/(P31) é exatamente o
    primo de Mersenne N = 2³¹−1.

## Problems This Solves

- **Problem:** o residual `R-crc` chamava de "axioma" uma conjunção em que
  uma parte (as classes) é teorema e outra (o canal) é axioma. Mistura
  dessas duas coisas é exatamente a classe de erro de claim que o 0061
  proíbe.
- **Problem:** não existia artefato formal nenhum do lado matemático do
  CRC (o Verus twin em `verus_crc_match.sh` prova que o código computa o
  CRC especificado, não que o CRC detecta classe alguma).
- **Problem:** "provar o CRC" era resposta binária ("impossível por
  contagem") — perdia-se o que **é** provável e útil.

## Proposed Solution

- Um arquivo Lean auto-contido, só Mathlib, zero `sorry`, zero `axiom`:
  `formal/aeneas/lean/CrcClass.lean` (81 teoremas), com os cinco teoremas
  de classe no fim:
  - `crc32c_burst_detected` — burst ≤ 32 bits;
  - `crc32c_odd_weight_detected` — peso ímpar;
  - `crc32c_two_bit_detected` — 2 bits, span < N;
  - `crc32c_weight_le_three_detected` — capstone peso ≤ 3, expoentes < N;
  - `crc32c_codeword_error_detected` — palavra-código + erro detectável
    permanece detectável.
- A ordem de X mod P31 = N **sem** API de grupo da Mathlib e **sem**
  irreducibilidade de P31: u^N = 1 vem de uma cadeia de Frobenius
  (squaring-and-multiplication: c₁ = X; c_{k+1} = spread(c_k)·X mod P31;
  c₃₁ = 1) montada por **30 certificados kernel-decidíveis** da forma
  `polyOf A ^ 2 * X + polyOf B = P31 * polyOf Q` (64 coeficientes cada,
  `decide`); u ≠ 1 vem do grau; ordem = N por minimalidade (`sInf`) +
  primalidade de 2³¹−1 (`native_decide`). Onde a cadeia para: irreduzível
  ou não, tudo o que o argumento precisa é u^N = 1 ∧ u ≠ 1 ∧ N primo.
- Re-scope do `R-crc` no catálogo: o `close` passa a enunciar o axioma de
  canal quantificado (a corrupção real cai na classe detectada) e a citar
  o teorema; `id`/`class`/`owner`/`never_floor` e todos os substrings
  que os testes Rust pinam ficam intactos.
- Nenhuma linha de código de runtime muda: o fail-closed do `crc_kernel`
  já é o comportamento; este RFC move fronteira de **conhecimento**, não
  de execução.

## Delivery slices (mandatory)

### P0 — o teorema de classe (útil sozinho: o que era "axioma" vira teorema)

- [x] **P0.1** `formal/aeneas/lean/CrcClass.lean`: fatoração G =
  (X+1)·P31 coeficiente-a-coeficiente; cadeia de Frobenius com 30
  certificados; ordem de X = 2³¹−1; os cinco teoremas de classe; zero
  `sorry`/`axiom`. Registro no gate: `CrcClass` no array COMPOSE de
  `scripts/lean_extracts.sh` + `[[lean_lib]]` no `lakefile.toml`.
  — status: `done`

### P1 — o residual honesto e os gates

- [x] **P1.1** `scripts/formal/residuals.json` R-crc re-escopado: axioma
  de canal quantificado citando RFC-0228; id/classe `never`/owner 0060/
  `never_floor` e os pins dos testes Rust
  (`crc_collision_axiom_remains`, `wal_open_crc_collision_axiom_remains`,
  `checkpoint_crc_collision_axiom_remains`, …) intactos. — status: `done`
- [x] **P1.2** Gates no mesmo commit, no que o RFC-0228 possui:
  `lake build CrcClass` verde (`CrcClass` registrado no gate
  `scripts/lean_extracts.sh --required` via COMPOSE + lakefile);
  `cargo test -p pedradb-core --lib --tests`: todos os pins CRC verdes
  (`crc_collision_axiom_remains`,
  `wal_open_crc_collision_axiom_remains`,
  `checkpoint_crc_collision_axiom_remains`, `vlog/history/manifest/
  changelog *_crc_collision_axiom_remains`) e todos os
  `crc_mismatch_on_live_*` fail-closed verdes;
  `./scripts/verus_crc_match.sh` verde. Pré-existentes na árvore no
  mesmo commit (fora do RFC-0228, trabalho in-flight da sessão
  paralela — nenhum arquivo Rust tocado por este RFC):
  `lean_extracts.sh --required` aborta no `Merge.lean` da re-extração
  em andamento (7 erros de prova, linhas 826–1057) antes de alcançar
  `CrcClass` no build completo; o alvo example `scan_readahead.rs` não
  compila (`OpenOptions.sst_warm_cap_bytes` nunca existiu; commit de
  snapshot a5ccc131); 32 testes de lib falham nos módulos em
  re-trabalho (changelog/bulk/history/prefix) — todos passam em
  isolamento quando o tema é I/O, e nenhum lê artefato deste RFC
  exceto os pins acima, que estão verdes.
  — status: `done` (superfície do RFC verde; blockers pré-existentes nomeados)

### P2 — depois / polimento

- [ ] **P2.1** Classes adicionais (peso 4 com span < N, bursts pareados,
  spans compostos) — só quando um claim de produto citar uma delas;
  sem claim, sem teorema. — status: `todo`
- [ ] **P2.2** none yet. — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | CrcClass.lean (5 teoremas de classe, 30 certs, 0 sorry) + gate COMPOSE/lakefile | done | este commit | 2026-09-15 |
| P1.1 | p1 | R-crc re-escopado para axioma de canal quantificado | done | este commit | 2026-09-15 |
| P1.2 | p1 | gates: lake build CrcClass verde; pins CRC e fail-closed verdes; verus verde (Merge.lean/example/32 testes pré-existentes quebrados na árvore, nomeados no slice) | done | este commit | 2026-09-15 |
| P2.1 | p2 | classes extra sob demanda | todo | — | 2026-09-15 |
| P2.2 | p2 | none yet | todo | — | 2026-09-15 |

## Acceptance Criteria

- **Tests**: `CrcClass` registrado no gate `scripts/lean_extracts.sh`
  --required` (COMPOSE + lakefile) e `lake build CrcClass` verde (zero
  `sorry`, `^theorem` presente); `cargo test -p pedradb-core --lib
  --tests` mantém `crc_collision_axiom_remains`,
  `wal_open_crc_collision_axiom_remains`,
  `checkpoint_crc_collision_axiom_remains` verdes (o axioma continua
  pinado no catálogo); `./scripts/verus_crc_match.sh` mantém o gêmeo
  Verus do crc32c (elo código↔matemática). O build completo do gate
  requer `Merge.lean` são da re-extração in-flight (pré-existente,
  sessão paralela) — ver P1.2.
- **Telemetry / Analytics**: none — artefato formal puro; nenhum caminho
  de runtime muda com este RFC.
- **Documentation**: este RFC, a linha em `docs/status.md` e o re-scope
  no `scripts/formal/residuals.json` no mesmo commit.
- **Screenshots**: backend-only.

## Out of scope

- Provar ausência total de colisões (falso por contagem — nunca será
  teorema, e nenhum texto deste repo pode alegar).
- Modelo de canal / ECC / bit-flip físico (fica em R-hardware; o
  `R-crc` re-escopado é a face de banco desse axioma).
- Mudar `crates/pedradb-core/src/wal/crc_kernel.rs` ou qualquer código
  de runtime (fail-closed já é o comportamento produtivo).
- Irredutibilidade de P31 (desnecessária para as classes provadas;
  registrar se algum dia P2.1 precisar).
