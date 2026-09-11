# RFC-0194 P0.1–P0.3 aterrizado: política de páginas leftover (advise off-lock) + telemetria opt-in

**Data:** 2026-09-10
**RFC:** [0194](../docs/rfc/0194-leftover-bounded-cache-25m.md) P0.1/P0.2/P0.3
**Host:** Darwin (DIAG dev). Meter Linux (P0.4) segue gate — ver abaixo.

## O que aterrizou

1. **Kernel puro (P0.1)** — `crates/pedradb-core/src/leftover_page_kernel.rs`
   (`#![forbid(unsafe_code)]` do crate intacto): `leftover_page_advice(budget,
   sst_bytes, warm_cap, covered)` = orçamento≠0→`KeepDefault`; SST viva cobre a
   família→`KeepCovered`; bytes ≤ warm cap→`KeepHot` (Fire 118: hot nunca
   solta); senão `Drop`. Gêmeo AS-IS (`leftover_page_advice_as_is` = sempre
   `KeepDefault`) + `family_upper_bound`/`sst_range_covers_family`. 7 testes.
2. **Wiring real (P0.2)** — decisão inteira sob o write lock nos DOIS sítios
   de instalação (`install_ssts_at_levels` = flush/serial;
   `install_prepared_l0_compact` = compactação), só **enfileirando** caminhos
   no `LeftoverAdviseBoard` (Arc compartilhado; mutex mínimo nunca segurado
   atravessando I/O). A decisão roda sobre o estado **pós-instalação** (`self.ssts`
   já contém os arquivos novos): a captura de caminhos é lazy ANTES do
   `apply_sst_installs` consumir as tabelas (só quando o budget está armado;
   default não captura, não aloca), mas o `note` vem DEPOIS de
   `apply_sst_installs` + `retire_flush_pin` — decisão pré-instalação veria
   `sst_bytes=0` no primeiro flush e adivinharia `KeepHot` errado (bug pego
   pelos testes `fires`/`is_off_lock`, corrigido 2026-09-10). Consequência
   visível: na iteração 1 do `fires`, o advise derruba a SST leftover da
   família estrangeira; na iteração 2 a SST própria da família viva é pulada
   como coberta (`keep_covered > 0` pinado). A I/O `Env::advise(DONTNEED)`
   drena **fora de qualquer lock do caminho de write**: janela lock-free do
   worker de flush do `ConcurrentDb::flush` (2 sítios: pós-`write_imm_l0_files`
   e pós-instalação), rabinho do `flush_imm_to_l0` serial e saídas do
   `compact_leveled` serial. Caminho rápido do drain: UM relaxed load em
   `pending_count` (config default nunca enfileira ⇒ nunca toma o mutex —
   a corrida do L0-stall é desse nível de aperto; medições 2026-09-10).
   Bytes de SST somados via `SstTable::payload_len_bytes()` (corpo do
   arquivo, zero stat I/O). Default (`sst_page_keep_budget = u64::MAX`) =
   comportamento de hoje exatamente — nada é aconselhado, E os 5 contadores
   ficam em zero (zero bookkeeping no caminho default), pinado por teste.
3. **Telemetria (P0.3)** — contadores por desfecho no board
   (`drop/keep_hot/keep_covered/keep_default/issued`, contagens de arquivo);
   `ConcurrentDb::io_advise_line()` imprime
   `leftover_advise=drop:N keep_hot:N keep_covered:N keep_default:N issued:N`
   somente sob `PEDRA_IO_ADVISE_STATS=1` (zero custo hot-path: bump só na
   instalação de SST). Bench imprime a linha ao lado do dump WRITEPHASE
   (traits `Engine::io_advise_line` no parity-bench + forward no
   `rocksdb_compat::DB`).

## Testes nomeados (verdes, single-thread)

- `leftover_advise_fires_only_in_bounded_cache` — bounded (cap 1 B, budget 0)
  emite DONTNEED só em `*.sst`; perna hot (cap `u64::MAX`) NÃO emite
  (regressão Fire 118 virada teste) e registra `keep_hot`.
- `leftover_advise_hot_never` — orçamento default: nem advise nem
  bookkeeping — os 5 contadores ficam em zero (o `note` nunca roda com
  fatia não-vazia; política desligada), e `env.dontneeds()` vazio.
- `leftover_advise_skips_covering_one_slash` — SST viva cobrindo a família ⇒
  sem advise; registra `keep_covered`.
- `leftover_advise_is_off_lock` — o drain exato do worker executado **com o
  write guard do `inner` seguro pelo próprio teste**: completa sem deadlock e
  emite (família de teste do ticket board 0193).
- `leftover_advise_counters_line_is_opt_in` — sem latch ⇒ `None`; com latch ⇒
  linha com todos os contadores.

## A/B serial (regra do gate)

Baseline pré-mudança capturado antes de qualquer edição (896 passaram / 21
falhas conhecidas / 4 ignoradas). Pós-mudança: **911 passaram / 21 falhas /
4 ignoradas — conjunto de falhas IDÊNTICO ao baseline (diff zero; +15 =
testes novos)** e musl `x86_64-unknown-linux-musl` `cargo check` exit 0.

## P0.3 em workload real (example `examples/leftover_advise.rs`)

Arquivos reais, flush worker real, `StdEnv` real, mesmo latch:

```
bounded leg line (latch unset): None
bounded leg line (latch on):    leftover_advise=drop:1 keep_hot:0 keep_covered:3 keep_default:0 issued:1
default leg line:              leftover_advise=drop:0 keep_hot:0 keep_covered:0 keep_default:0 issued:0
```

Perna drop = famílias seedadas NA MESMA memtable da família nova (shape do
teste `fires` — sem SST viva prévia cobrindo); perna covered = segundo
round com a família ycsb/ já viva. Default: zero bookkeeping. Nota Darwin:
o efeito de eviction de páginas precisa do page cache Linux (DIAG); a
decisão/queue/issue/contadores são o caminho enviado.

## P0.4 (meter 15M/25M) — gate

Segue o gate do host (guest `linux-gate-p149b` resolúvel ∧ Darwin load1 < 8).
Bloqueado ⇒ Darwin DIAG + blocked datado, nunca cartaz (precedente
0190/0193). Ver rfc0193-p05-meter-blocked.md e a checagem de gate desta onda.
