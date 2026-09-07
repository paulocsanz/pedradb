# RFC: translation validation rustc→objeto (um alvo pinado)

**Status:** done
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

- [x] **P0.1** Pin do triple + rustc version no findings; script `scripts/tv_write_admission.sh` recusa toolchain drift — status: `done`
- [x] **P0.2** Dump do objeto / LLVM IR do kernel isolado (crate Aeneas já isola o path) — status: `done`

### P1 — correspondência

- [x] **P1.1** Relação nomeada exec-fn → símbolo no objeto para `write_admission_idle` e `write_admit` — status: `done`
- [x] **P1.2** Gate CI: o dump muda se o kernel mudar (sha256 bind como SOURCE.*) — status: `done`

### P2 — TV mecânico

- [x] **P2.1** Ferramenta TV (Alive2 / LLVM-reduce / script próprio) sobre **um** fn — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | pin triple+rustc | done | findings/2026-09-06-rfc0172-tv/pin.txt | 2026-09-06 |
| P0.2 | p0 | dump objeto/IR | done | findings/2026-09-06-rfc0172-tv/write_admission.ll | 2026-09-06 |
| P1.1 | p1 | símbolos idle/admit | done | scripts/tv_write_admission.sh | 2026-09-06 |
| P1.2 | p1 | sha256 bind | done | findings/2026-09-06-rfc0172-tv/SOURCE.tv | 2026-09-06 |
| P2.1 | p2 | TV de um fn | done | scripts/tv_write_admission_ir.py (idle 8/8) | 2026-09-06 |

## Acceptance Criteria

- **Tests:** `scripts/tv_write_admission.sh` falha se `rustc -vV` ≠ pin, se o IR emitido ≠ dump, se faltam símbolos `write_admission_idle`/`write_admit`, ou se o IR de `idle` ⊭ spec nas 8 entradas.
- **Telemetry / Analytics:** none.
- **Documentation:** este RFC; 0171 P2.2 aponta para aqui.
- **Screenshots:** backend-only.

## Out of scope

- Tirar `R-rustc` do `never_floor`.
- CompCert / prova do LLVM.
- Todos os kernels; todos os alvos.
- `fsync` / CPU / Z3.
