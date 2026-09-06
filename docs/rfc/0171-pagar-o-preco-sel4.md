# RFC: pagar o preço seL4 (o artefacto que corre *é* o da prova)

**Status:** done
**Updated:** 2026-09-06
**Parents:** [0061](0061-residuals-sel4-ironfleet.md), [0170](0170-refinamento-sel4-class.md), [0056](0056-one-hundred-percent-delivery.md)
**Dono:** decisão do autor neste dia — disposto a pagar o que seL4 pagou, não a recusa L46.

**Frase deste RFC:** o kernel de produção que o binário liga é o termo que o prover vê. Sem ficheiro-gémeo ao lado. Glue só no sítio onde seL4 também teve assembly: `Env` / syscall / trampoline.

**Ainda recusado (seL4 também não provou):** “sem bugs”, CPU, rustc *em geral*, Z3/Isabelle, colisão CRC, mídia `fsync`. `R-cpu` `R-rustc` `R-verus` `R-crc` `R-deps` **ficam** `never`. Só `R-extract` sai do `never_floor`.

## Background

- seL4 não “não teve TCB”. Teve CPU, gcc, Isabelle, boot. Pagou outra coisa: ~10 kLOC de C **escritos para a prova**, ~200 kLOC Isabelle, o C que corre refinado à spec. Depois, num ARM, translation validation do binário (não provaram o gcc).
- RFC-0170 paga a *classe* da garantia **nas decisões**, gémeo ao lado, `db.rs` intocado como ficheiro. Isso não é o preço seL4.
- L46 / RFC-0061 out-of-scope diziam “não reescrever o engine no prover”. Essa linha era a recusa de pagar. Este RFC **revoga** essa recusa.
- Não vamos dump `db.rs` no Charon (`glue.db_rs_extracted` continua `false`). seL4 também não meteu 13 kLOC de glue no Isabelle: escreveram um kernel pequeno o suficiente. O preço é **esvaziar** o glue até só restar trampoline, não promover o glue.

## Problems This Solves

- **Problem:** “como atingimos o nível seL4” estava a ser respondido com a recusa, não com o mapa do preço.
- **Problem:** `R-extract=never` torna o preço ilegal no freeze.
- **Problem:** gémeo Verus ≠ produção. seL4 não tinha `verus/foo.rs` ao lado de `foo.rs`.

## Proposed Solution

O preço, na ordem em que seL4 o pagou:

1. **Um artefacto.** O `.rs` que o `rustc` compila é o que o Verus/Aeneas vê. Gémeo-cópia acaba (produção `#[cfg]` / o path do crate *é* o ficheiro verificado).
2. **Kernel pequeno o suficiente.** Cada `if` de destino de dados sai de `db.rs` para esse artefacto. O que resta em `db.rs` é `Env` (open/read/write/`fdatasync`) — o assembly deles.
3. **Spec ↔ exec.** Já temos close+extract por fn (0170). Este RFC exige isso **no ficheiro que liga**, não num twin.
4. **Binário (P2, como eles no ARM).** Translation validation de *um* rustc/LLVM pinado num alvo. Não é provar o rustc. `R-rustc` só mexe com RFC filho.

IronFleet/Dafny-rewrite não é este RFC (loop principal no TCB). CompCert do rustc não é P0.

## Delivery slices (mandatory)

### P0 — a recusa cai e o primeiro artefacto é um só ficheiro

- [x] **P0.1** Este RFC: o preço nomeado (artefacto único; glue = trampoline; binário depois) — status: `done`
- [x] **P0.2** `R-extract` sai de `never_floor` / classe `never` → `open`, owner 0171; RFC-0061 never-list e out-of-scope no mesmo commit — status: `done`
- [x] **P0.3** Um kernel já close (candidato: `write_admission_kernel.rs`) passa a ser verificado **no ficheiro de produção** (Verus no path que o crate liga; o twin-cópia ou some ou vira include). `--lint` recusa twin path ≠ kernel path nesse par — status: `done`

### P1 — o put-Ok até o trampoline

- [x] **P1.1** Caminho `put` → WAL append → barreira → ack: zero `if` de destino de dados em `db.rs` (só chamadas a kernels do artefacto único) — status: `done`
- [x] **P1.2** Recover/reopen do mesmo caminho, mesmo critério — status: `done`
- [x] **P1.3** Aeneas do artefacto do P0.3 / P1.1 (produção = extract, sha256) — status: `done`

### P2 — o que seL4 pagou *depois* do C

- [x] **P2.1** Inventário do trampoline restante em `db.rs` (I/O `Env` only) + freeze `handler_loc` a descer — status: `done`
- [x] **P2.2** RFC filho: translation validation rustc→objeto num alvo pinado (equivalente ao gcc ARM deles). `R-rustc` permanece `never` até esse RFC fechar um alvo, não “o rustc” — status: `done` ([0172](0172-rustc-translation-validation-one-target.md))

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | mapa do preço | done | this RFC | 2026-09-06 |
| P0.2 | p0 | R-extract never→open | done | this RFC | 2026-09-06 |
| P0.3 | p0 | Verus no ficheiro que liga | done | this RFC | 2026-09-06 |
| P1.1 | p1 | put-Ok sem if em db.rs | done | this RFC | 2026-09-06 |
| P1.2 | p1 | recover do mesmo caminho | done | this RFC | 2026-09-06 |
| P1.3 | p1 | Aeneas do artefacto único | done | this RFC | 2026-09-06 |
| P2.1 | p2 | trampoline Env only | done | findings/2026-09-06-rfc0171-trampoline/ | 2026-09-06 |
| P2.2 | p2 | binary TV num alvo | done | [0172](0172-rustc-translation-validation-one-target.md) | 2026-09-06 |

## Acceptance Criteria

- **Tests**
  - P0.2: `python3 scripts/formal/test_residuals_freeze.py` verde; `never_floor` não contém `R-extract`; RFC-0061 lista os five never que restam e aponta 0171 para extract.
  - P0.3: `verus <production kernel.rs>` verde; lint do par falha se `twin` ≠ `kernel`.
  - P1.1: grep de `if` de destino de dados no `put` path em `db.rs` = vazio (teste nomeado).
- **Telemetry / Analytics:** none — prova; o sinal é exit code + stamp.
- **Documentation:** este RFC; 0061 never-list; linha `docs/status.md`.
- **Screenshots:** backend-only.

## Out of scope

- Dump de `db.rs` como um kernel (`glue.db_rs_extracted` fica `false`).
- Apagar `R-cpu` / `R-verus` / `R-crc` / `R-deps` / `R-rustc` (este último só com o RFC filho P2.2, e só num alvo).
- Provar Linux / `fsync` de mídia / ring io_uring.
- `∀` interleavings de `ConcurrentDb`.
- Reescrever em Isabelle/C ou Dafny (o preço seL4 aqui é Rust verificado = produção, não mudar de língua).
- Benches / crates.io.
