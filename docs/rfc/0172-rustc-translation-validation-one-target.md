# RFC: translation validation rustc→objeto (um alvo pinado)

**Status:** draft
**Updated:** 2026-09-06
**Parent:** [0171](0171-pagar-o-preco-sel4.md)

**Residual:** `R-rustc` **permanece `never`**. Este RFC não prova o rustc. Fecha *um* alvo: o objeto que o rustc/LLVM pinado emite para `write_admission_kernel.rs` corresponde ao termo que Verus/Aeneas viram.

## Background

- seL4, depois do C, pagou translation validation do binário num ARM com um gcc pinado — não provaram o gcc.
- RFC-0171 P2.2: o mesmo passo para Pedra, num alvo, sobre o artefacto único (produção = prova).
- `never_floor` continua a listar `R-rustc`. Só um RFC futuro que feche o alvo (não “o rustc”) pode discutir a classe.

## Problems This Solves

- **Problem:** 0171 P0–P1 param no Rust. seL4 pagou o passo extra até o objeto.
- **Problem:** sem alvo pinado, “TV do rustc” vira slogan.

## Proposed Solution

Um alvo, um crate, um ficheiro:

- Host/alvo: aarch64-apple-darwin **ou** x86_64-unknown-linux-gnu (o que o CI de prova já corre).
- rustc/LLVM: versão pinada no `rust-toolchain.toml`.
- Objecto: `write_admission_kernel` (o ficheiro 0171 P0.3).
- Método: translation validation (MIR/LLVM bitcode vs o extract Lean/Verus), não CompCert do rustc.

## Delivery slices (mandatory)

### P0 — o alvo existe no papel e no freeze

- [ ] **P0.1** Pin do triple + rustc version no findings; script `scripts/tv_write_admission.sh` recusa toolchain drift — status: `todo`
- [ ] **P0.2** Dump do objeto / LLVM IR do kernel isolado (crate Aeneas já isola o path) — status: `todo`

### P1 — correspondência

- [ ] **P1.1** Relação nomeada exec-fn → símbolo no objeto para `write_admission_idle` e `write_admit` — status: `todo`
- [ ] **P1.2** Gate CI: o dump muda se o kernel mudar (sha256 bind como SOURCE.*) — status: `todo`

### P2 — TV mecânico

- [ ] **P2.1** Ferramenta TV (Alive2 / LLVM-reduce / script próprio) sobre **um** fn — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | pin triple+rustc | todo | — | 2026-09-06 |
| P0.2 | p0 | dump objeto/IR | todo | — | 2026-09-06 |
| P1.1 | p1 | símbolos idle/admit | todo | — | 2026-09-06 |
| P1.2 | p1 | sha256 bind | todo | — | 2026-09-06 |
| P2.1 | p2 | TV de um fn | todo | — | 2026-09-06 |

## Acceptance Criteria

- **Tests:** script de pin falha se `rustc -vV` ≠ freeze; dump existe no findings.
- **Telemetry / Analytics:** none.
- **Documentation:** este RFC; 0171 P2.2 aponta para aqui.
- **Screenshots:** backend-only.

## Out of scope

- Tirar `R-rustc` do `never_floor`.
- CompCert / prova do LLVM.
- Todos os kernels; todos os alvos.
- `fsync` / CPU / Z3.
