# RFC: 0232 — Trampolim só Env: o glue que resta depois do TCB hospedeiro congelado

**Status:** in-progress
**Updated:** 2026-09-16
**ID:** 0232
**Parents:** [0229](0229-host-tcb-media-sched-pct-stdenv-liveness.md) (host TCB
congelado; grant token; N=3; ES Lean),
[0227](0227-prova-de-produto-put-get-recover-replica.md) (`pedra_refines`),
[0171](0171-pagar-o-preco-sel4.md) (o termo é o rustc; glue = trampoline Env),
[0219](0219-trampolim-vira-escada-um-if-data-fate-por-kernel.md)
**Peer:** inalterado — RocksDB default `sync=false`. Este RFC não toca parity.

> `leftover_next none`. D1/R1/T1/C1 estão no Lean. O host TCB está
> escrito e as admissions são `false`. O que resta no *lado Pedra* não
> é flipar `fdatasync` — é esvaziar o glue: `db_kernel` / `concurrent_kernel`
> só chamam planos nomeados e Env. Rel cita o grant token. O extract de
> `group_commit` volta a ser gerado, não escrito à mão.

**Frase permitida no fim deste RFC** (relativa ao TCB): *o trampolim
do writer/get/recover/replica não contém `if` de destino de dados; só
syscall Env e `match` de plano extraído. `pedra_refines` unfold
`write_group_wait_grant`. Admissions host intactas.*

**Frases recusadas:** “somos seL4”, “o fsync está provado”, “extraímos
`db.rs`”, “∀π do Linux”.

## Background

Medido 2026-09-16 após RFC-0229 `done`:

| o quê | hoje | o que isso **não** é |
|---|---|---|
| leftover_next | none; unpaid_product=0; unpaid_trampoline_data_fate_ifs=0 | glue Env ainda existe (`candidates.py`: “Env glue remains”) |
| A2a | kernel_loc/(kernel_loc+handler_loc) ≈ 58% | 100% seL4; handler_loc ainda ~120 kLOC |
| Rel | `pedra_refines` único; `inv_wal`+`get_live` | não unfold `write_group_wait_grant` |
| group_commit extract | Kernel gerado + defs P1.1/P1.2 **à mão** no fim de `GroupCommitKernel.lean` | extract Aeneas do rustc para o grant/N=3 |
| ConcurrentDb parks | `granted_block` / `granted_sleep` | Isolated extract do wait; dump de `concurrent.rs` |
| Host TCB | `media_durable_admitted`/`lock_interleavings_admitted`/`forall_schedules_admitted` = false | nunca flipar |

O degrau drástico depois de congelar o TCB do host é o preço 0171:
produção Rust, kernels extraídos, trampolim = Env. Enquanto
`handler_loc` cresce com `match` de plano, A2a sobe de verdade. Enquanto
o grant token só existe no rustc e num def manuscrito, o Rel mente.

## Problems This Solves

- **Problem:** “acabou a verificação” depois de 0229 arredonda Env glue
  e defs Lean manuscritos a prova de implementação.
- **Problem:** `pedra_refines` não cita o grant que o wait agora chama.
- **Problem:** re-extract de `group_commit` hoje perderia os defs P1.1/P1.2.

## Proposed Solution

1. Inventário vivo: cada `if` em `db_kernel.rs` / `concurrent_kernel.rs`
   que não é `match` de plano nem syscall Env entra num TSV; sítio novo
   = vermelho (mesmo dente do park TSV).
2. `pedra_refines` / `Rel` dual-unfold `write_group_wait_grant` ×
   `lock_alphabet_linearizes_n2` (já teorema em GroupCommit).
3. Re-extract Aeneas de `group_commit_kernel.rs` no pin actual; os defs
   manuscritos saem; `lake build GroupCommit` duas vezes.
4. Não dump. Não flipar admissions. Não Isolated-dyn.

## Delivery slices (mandatory)

### P0 — must ship first

- [x] **P0.1** TSV + checker `--selftest`: `if` de destino de dados em
  `db_kernel.rs`/`concurrent_kernel.rs` (produção, antes de `mod tests`)
  que não chama um plano nomeado. Floor exact-count. — status: `done`
  (`scripts/check_trampoline_glue.py` + job `trampoline-glue`; leftover
  2 sítios pinados: `refuse_publish`, unpublished-below-floor;
  `--selftest` 4/4)
- [ ] **P0.2** `pedra_refines` (ou corolário no mesmo ficheiro) unfold
  `write_group_wait_grant` e `lock_alphabet_linearizes_n2`. A1 continua 1.
  — status: `doing`

### P1 — next wave

- [ ] **P1.1** Re-extract `group_commit` (Charon 0.1.232 / Aeneas
  `daa85d7`); defs manuscritos de `WriteGroupWait` / `n3` saem do
  `GroupCommitKernel.lean` gerado. `lake build GroupCommit` ×2.
  — status: `todo`
- [ ] **P1.2** Um `if` do TSV P0.1 vira `match` de plano extraído
  (produção + Lean unfold). — status: `todo`

### P2 — later

- [ ] **P2.1** Isolated walk do `granted_block` se Charon recusar o glue
  do wait (residual nomeado em EXTRACT, não dump). — status: `todo`
- [ ] **P2.2** A2a só sobe com handler_loc a descer ou kernel_loc a
  cobrir o mesmo handler (gate sel4_gap). — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | TSV ifs glue sem plano | done | `check_trampoline_glue.py` | 2026-09-16 |
| P0.2 | p0 | Rel unfold grant × alfabeto | doing | — | 2026-09-16 |
| P1.1 | p1 | re-extract group_commit | todo | — | 2026-09-16 |
| P1.2 | p1 | um if do TSV → match plano | todo | — | 2026-09-16 |
| P2.1 | p2 | Isolated granted_block ou residual | todo | — | 2026-09-16 |
| P2.2 | p2 | A2a honesto (handler_loc) | todo | — | 2026-09-16 |

## Acceptance Criteria

- **Tests**
  - P0.1: `--selftest` apanha if novo e if removido; live GREEN.
  - P0.2: `sel4_gap` A1=1; grep `write_group_wait_grant` no teorema Rel.
  - P1.1: `SOURCE.group_commit` sha do rustc; Kernel sem bloco “hand-written”.
  - P1.2: cargo no handler + lake unfold.
- **Telemetry / Analytics** — none — CI vermelho/verde.
- **Documentation** — este RFC; linha em `docs/status.md`; EXTRACT datada.
- **Screenshots** — none (backend/CI-only).

## Out of scope

- Flipar `media_durable_admitted` / `lock_interleavings_admitted` /
  `forall_schedules_admitted`.
- Dump de `db.rs` / `concurrent.rs`. Provar Linux, rustc, firmware.
- Liveness sem ES. Montanha FDB. Perf / Rocks parity.
- “Somos seL4”.
