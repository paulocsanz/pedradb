# RFC-0199 P2.1 + P2.2 — ferramenta de derivação + fronteira fdatasync por classe de host

**Date:** 2026-09-11 · **RFC:** docs/rfc/0199-escada-de-contagem-complexidade-verificada.md

## P2.1 — derivação mecânica de anotações de custo (ferramenta própria)

`scripts/ratchet/derive_count_annotations.py` — pós-processador mantido no
repo sobre os extracts Aeneas gerados (o pin Aeneas não é forkado; a
ferramenta é nossa e re-executável). Para cada fn inscrita ele conta
textual e deterministicamente o trabalho de UMA expansão:

- `leaf_calls`: chamadas a `axiom`s do próprio arquivo (folhas externas
  não traduzidas — `any`, `saturating_mul`, …)
- `local_calls`: chamadas a outros `def`s do arquivo (custo nível
  construtor, 1 unidade cada)
- `dispatches`: `match` + `if`
- `cmp_ops`: `<= >= < > ≠ =` (exclui `:=`, `=>`, shifts)
- `arith_ops`: `<<<` e ` + ` ` - ` ` * `
- `nested_calls` (loops inscritos aninhados) e `self_calls`: contados,
  EMITIDOS, e EXCLUÍDOS do `step_work` — um sub-loop é cobrado pelo seu
  teorema `count` registrado, nunca achatado numa constante

Saída: `formal/aeneas/lean/CountDerived.lean` (AUTO-GENERATED) — defs
`Nat` + teoremas (`_step_work_eq` por rfl, `_step_work_positive` por
decide, `all_enrolled_step_work_positive`), no mesmo build lake dos
extracts (COMPOSE). `scripts/lean_extracts.sh` roda `--check` ANTES do
build: anotação dessincronizada com o extract falha o gate lean.

Inscritas (6): `auto_flush_due` (FlushKernel), `scan_kernel.scan_reads_file`
(ScanKernel), `lsm_compact_src_loop` + `lsm_compact_inner_loop` (LsmR1Kernel),
`probe_order_covering_loop` (ProbeOrderKernel), `level_count_loop`
(ScaleKernel). Derivado (verificado à mão contra os corpos):
2 / 4 / 5 / 10 / 13 / 5 unidades por expansão.

Demo de mordida (2026-09-11, log `p21_derive.log` no scratch da sessão):
uma aritmética a mais em `lsm_compact_inner_loop.body` (cópia scratch) →
`arith_ops` 3→4, `step_work` 10→11 no arquivo regenerado; diff não-vazio.
Re-executável: `python3 scripts/ratchet/derive_count_annotations.py`.

## P2.2 — fronteira fdatasync no modelo, por classe de host

O extract posix NÃO modela a barreira como construtor contável — o modelo
é nosso, na álgebra `Work.io` (P1.1): `Work.fdatasync` + ponte
`fdatasync_rc_ok_iff_zero` sobre o extract `PosixKernel` (rc=0 único ok).
P2.2 fecha a dimensão de CLASSE DE HOST (`WorkIo.lean`):

- `HostIoClass` (`linux_fdatasync` | `darwin_fullfsync`) — a CLASSE não
  entra na contagem: `barrier_work_is_the_counted_constructor`,
  `barrier_count_class_independent` (1 barreira em toda classe)
- `wal_commit_work_on` + `wal_commit_work_on_eq`: o plano indexado por
  classe é o mesmo trabalho contável
- `wal_commit_plan_committed_sync_count_any_class` e
  `wal_commit_plan_at_most_one_fdatasync_any_class`: os teoremas P1.1
  valem verbatim em toda classe

### Registro de âncoras ns datadas (medidas — nunca teorema)

| Classe | Semântica (datada) | Âncora ns | Status |
|---|---|---|---|
| `linux_fdatasync` | `fdatasync(2)` (Rocks CMake default no Linux) | `LINUX_QUIET_0189_P01` (RFC-0189 P0.1, 2026-09-10, linux p149b quiet, overwrite_mc4) — âncora de FASES por op consumida por `write_cycle_kernel`; **sem slice de barreira por op** (barreira é por grupo confirmado, ≤1 pelo teorema) | registrada, datada |
| `darwin_fullfsync` | `fcntl(F_FULLFSYNC)` — o que CMake Rocks faz no macOS (`findings/2026-08-27-upstream-fullfsync/`, 2026-08-27) | — | **aberta**: sem medição quiet datada do custo ns da barreira darwin ainda; veículo = nightly experimental RFC-0187 P2.2 / run quiet darwin |

A contagem NÃO depende da âncora aberta: os teoremas any-class são sobre
o construtor contável; ns por classe permanece âncora medida e datada
(RFC-0187: persistência física é experimento/TCG — `F_FULLFSYNC` inclusive
verifica só até o contrato do disco, nunca a física).
