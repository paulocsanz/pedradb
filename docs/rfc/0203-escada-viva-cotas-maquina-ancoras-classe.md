# RFC: 0203 — escada viva: cotas mantidas por máquina, âncoras por classe de host

**Status:** draft
**Updated:** 2026-09-11
**Parents:** [0199](0199-escada-de-contagem-complexidade-verificada.md) (a escada
completa 10/10: 7 linhas `count`, floor 7, inventário sem `todo`),
[0187](0187-teorema-experimento-tcb.md) (âncora ns nunca é teorema; F_FULLFSYNC
físico segue experimento), [0192](0192-write-cycle-forecast.md) (o kernel de
forecast que consome contagens × âncoras — arquivo da sessão paralela, não
editado aqui), [0202](0202-quatro-teoremas-concorrencia.md) (fila de
concorrência; não sobrepõe)

Nota de régua: fatias de prova e âncoras medidas; nenhum claim de perf ou
cartaz. Cartaz continua sendo Pedra vs RocksDB default `sync=false`
(`ROCKS_PARITY_SYNC=0`). Previsão é hat com erro nomeado.

> **Tese:** o 0199 provou as cotas de trabalho e as registrou (7 linhas
> `count`, `floor_count` 7, gates verdes) — mas a MANUTENÇÃO da escada é
> manual em três pontos que a máquina pode e deve fechar, e cada um deles
> já deixou registro de dívida no próprio repo: (1) o twin é um espelho
> hand-escrito — o header do `LsmCompactCount.lean` declara "count twins
> are hand-written Nat mirrors … declared debt until the P2.1 cost tool
> derives them mechanically", e a ferramenta P2.1 existe (`derive_count_
> annotations.py`) sem que nenhum contrato a ligue aos twins; (2) a
> terminalidade do inventário foi um SWEEP manual (0199 P2.3) — nada
> impede que um kernel novo entre em `todo` e ninguém perceba; (3) a
> âncora darwin `F_FULLFSYNC` ficou OPEN (findings 2026-09-11: "sem
> medição quiet datada … veículo = nightly 0187 P2.2") enquanto a linux
> está datada desde 0189. Este RFC paga os três: contrato de twin
> derivado e gateado, terminalidade virando gate, âncora darwin medida e
> datada com rótulo honesto — e uma tabela de âncoras por classe de host
> que o tie do forecast consome sem editar o kernel da sessão paralela.

## Background

- A escada (`docs/verification-ledger.md` §"Escada de contagem") tem hoje
  7 linhas `count` terminais: probe ladder, escala, compact one-pass,
  memtable→flush amortizado, write path confirmado (≤1 fdatasync/grupo),
  scan linear, bloom probe. `floor_count` 7; residuals `count` 7; gates
  `depth-floor` (selftest 5/5), `ledger`, `product-floor` verdes no fechamento
  `1c292134`.
- A ferramenta P2.1 (`scripts/ratchet/derive_count_annotations.py`) deriva
  anotações de custo por expansão das 6 fns inscritas e emite
  `CountDerived.lean` com `--check` dentro de `lean_extracts.sh` — mas nada
  conecta essas anotações aos twins Rust: cada twin afirma seu bound
  independentemente, e a equivalência twin↔extract vive na leitura humana.
- A tabela de âncoras hoje: linux = `LINUX_QUIET_0189_P01` (RFC-0189 P0.1,
  2026-09-10, p149b quiet, fases por op, sem slice de barreira por op —
  barreira é por grupo confirmado); darwin = semântica datada 2026-08-27
  (`fcntl(F_FULLFSYNC)`, o que CMake Rocks faz no macOS — o rust-rocksdb
  crates.io NÃO define `HAVE_FULLFSYNC`), custo ns **aberto**.
- Os teoremas any-class do `WorkIo.lean` (`wal_commit_plan_at_most_one_
  fdatasync_any_class` etc.) tornam a CONTAGEM independente de classe — a
  âncora por classe é o único lado aberto, e é medida, nunca teorema.

## Problems This Solves

- **Problem:** um twin pode divergir do extract (renome, reescrita do loop,
  mock da fn) e nenhum gate percebe — a amarração é textual e revisada à mão.
- **Problem:** um kernel novo no inventário pode entrar `todo` e ficar — a
  regra "todo→count exige teorema+twin+linha+floor no mesmo commit" não tem
  gate de terminalidade.
- **Problem:** o forecast compõe contagens × âncoras, mas só existe âncora
  datada para linux; qualquer raciocínio darwin usa número de outra classe
  ou de máquina não-quiet sem registro.

## Proposed Solution

- Contrato de twin **derivado e gateado**: a ferramenta emite um TSV
  ligando cada linha `count` → teste twin → fn de produção dirigida →
  anotação derivada; um novo gate verifica completude, que o twin chama a
  fn real e que a anotação está em sync (`--check`).
- Terminalidade do inventário **como gate**: parse da tabela do ledger;
  toda linha `count` ou `deferido <data> <motivo>`; `todo` é vermelho.
- Âncora darwin **medida e datada** nesta máquina (darwin arm64), com
  loadavg registrado e rótulo honesto (`quiet` ou `DIAG` conforme o load
  no momento) — medir ÂNCORA do primitivo `fcntl(F_FULLFSYNC)` vs
  `fdatasync` intra-host; não reabre durabilidade (0187).
- Tabela de âncoras **por classe** em `scripts/ratchet/host_anchors.tsv`,
  consumida por teste (completude + datas), sem editar o
  `write_cycle_kernel` (arquivo in-flight da sessão paralela).

## Delivery slices (mandatory)

### P0 — must ship first (smallest vertical slice that is useful)

- [x] **P0.1** Contrato de twin derivado: `derive_count_annotations.py`
  ganha emissão de `scripts/ratchet/twin_contracts.tsv` — uma linha por
  linha `count` (par, teorema, caminho do twin test, fn de produção
  dirigida — para `lsm_compact` o kernel Rust espelho `lsm_r1_kernel`,
  registrado como tal, não o walk Charon-refused), step_work derivado — e
  novo gate `scripts/check_twin_contracts.py`: toda linha `count` tem
  contrato; o twin existe e chama a fn de produção nomeada (nenhum mock da
  unidade sob teste); `CountDerived.lean` em sync; selftest sabota
  (twin renomeado, fn trocada por mock, contrato faltando) e pega tudo —
  status: `done` (2026-09-11; 7/7 linhas bound — 5 pares com step_work
  derivado em `CountDerived.lean`, 2 com teorema de mão registrado como
  tal — `wal_commit_plan`@WorkIo, `bloom_may_contain`@BloomCount —;
  TSV byte-idêntico à emissão da ferramenta, edição à mão = RED;
  selftest 6/6; job `twin-contracts` no `verification-gates.yml`)
- [x] **P0.2** Terminalidade do inventário como gate:
  `scripts/check_inventory_terminal.py` — parse da tabela de inventário no
  `docs/verification-ledger.md`; toda linha termina `count` (com teorema
  registrado no ratchet) ou `deferido` com data+motivo; qualquer `todo`
  falha o gate; selftest com linha `todo` plantada pega — status: `done`
  (2026-09-11; 7 linhas count todas registradas, bidirecional — teorema
  sem linha no inventário também é RED; selftest 5/5 inclui par novo
  `todo` e par anônimo sem `catalog:`; linha auto_flush_due ganha seu
  `catalog:auto_flush_due` no MESMO commit; job `inventory-terminal` no
  `verification-gates.yml`)

### P1 — next wave (depends on P0 or clearly deferrable)

- [x] **P1.1** Âncora darwin `F_FULLFSYNC` medida e datada: medição do
  primitivo nesta máquina (darwin arm64) — barreira `fcntl(F_FULLFSYNC)`
  vs `fdatasync` intra-host, N repetições, loadavg registrado no momento,
  rótulo `quiet`/`DIAG` honesto; registrada em findings com data e método;
  nunca teorema de ns (0187 intocado: persistência física segue
  experimento; o crates.io rust-rocksdb continua sem `HAVE_FULLFSYNC` —
  contexto do finding 2026-08-27 citado, sem re-bench do Rocks) —
  status: `done` (2026-09-11; `crates/pedradb-posix/examples/fullfsync_anchor.rs`
  re-executável, 2×200 barreiras/sabor: fdatasync p50 17,7/19,8 µs,
  F_FULLFSYNC p50 4,00/4,08 ms, multiplicador 206–227×; loadavg 10–16
  nos dois runs ⇒ rótulo **DIAG** honesto;
  `findings/2026-09-11-rfc0203-p11-darwin-fullfsync-anchor.md`)
- [x] **P1.2** Tabela de âncoras por classe consumida:
  `scripts/ratchet/host_anchors.tsv` (classe, âncora, valor ns, data,
  host, rótulo quiet/DIAG, fonte) com linux = referência à
  `LINUX_QUIET_0189_P01` (sem duplicar valor em Lean) e darwin da P1.1;
  teste de consumo (estende o tie do 0199 ou novo teste): ambas as classes
  presentes e datadas, toda linha tem fonte, e os multiplicadores count
  seguem class-independent citando os teoremas any-class do `WorkIo.lean`
  — o kernel da 0192 não é editado — status: `done` (2026-09-11; 3 linhas
  — linux_fdatasync quiet (pino de fases por op, labeled-stale pré-0193
  na fonte), darwin_fdatasync + darwin_fullfsync DIAG do P1.1; consumo
  `crates/pedradb-core/tests/host_anchor_table.rs` (2 tests: classes
  datadas+fontadas+grammar valor; teoremas any-class presentes no
  WorkIo.lean); ponteiro vivo no ledger §Escada de contagem; nenhum
  arquivo Lean tocado)

### P2 — later / polish

- [ ] **P2.1** Cadência de novos pares gateada: a regra do ledger
  ("Movimento de linha") vira enforcement — um par novo no inventário sem
  linha `count` ou `deferido` datado no MESMO commit falha o
  `check_inventory_terminal` (gate já pago na P0.2; aqui documenta-se o
  rito no runbook de gates e no texto do ledger, e o selftest cobre o caso
  do par novo) — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | Contrato de twin derivado + gate (selftest) | done | 2026-09-11 | 2026-09-11 |
| P0.2 | p0 | Terminalidade do inventário como gate | done | 2026-09-11 | 2026-09-11 |
| P1.1 | p1 | Âncora darwin F_FULLFSYNC medida e datada | done | 2026-09-11 | 2026-09-11 |
| P1.2 | p1 | Tabela de âncoras por classe consumida por teste | done | 2026-09-11 | 2026-09-11 |
| P2.1 | p2 | Cadência de novos pares gateada (rito no runbook) | todo | — | 2026-09-11 |

## Acceptance Criteria

- **Tests:** `check_twin_contracts.py` e `check_inventory_terminal.py`
  verdes no commit de cada fatia, cada um com selftest que planta a
  sabotagem (contrato faltante/twin renomeado/fn mockada; linha `todo`
  plantada) e pega; `lean_extracts.sh --required` segue verde
  (`--check` do derive incluído); teste de consumo da tabela de âncoras
  (P1.2) verde com ambas as classes datadas.
- **Telemetry / Analytics:** none — por quê: fatias de prova e âncoras
  datadas; os números vivem em TSVs versionados (`twin_contracts.tsv`,
  `host_anchors.tsv`, ratchet) e findings datados, não em métricas de
  runtime.
- **Documentation:** checkbox + status table deste RFC no mesmo commit da
  fatia; `docs/verification-ledger.md` (tabela de âncoras e rito do
  "Movimento de linha") no commit da P1.2/P2.1; findings datado da
  medição darwin no commit da P1.1.
- **Screenshots:** backend-only (gates Lean/Rust + TSVs).

## Out of scope

- Teorema de ns ou de durabilidade física: âncora é medida datada com
  rótulo; TCG power-cut e `F_FULLFSYNC` físico seguem experimento 0187 —
  medir âncora não reabre a fronteira.
- Editar `write_cycle_kernel.rs` / `concurrent.rs` ou qualquer arquivo
  in-flight da sessão paralela (0192–0196): o consumo da tabela de
  âncoras é por teste, nunca por edição do kernel.
- Novos teoremas `count` para kernels novos (seguem o rito 0199 por fire;
  este RFC gateia a cadência, não paga cotas novas).
- Claim de perf/cartaz: cotas e âncoras não são wins; a régua Rocks
  default `sync=false` (`ROCKS_PARITY_SYNC=0`) segue intocada; comparação
  darwin não é cartaz (DIAG salvo rótulo quiet registrado).
