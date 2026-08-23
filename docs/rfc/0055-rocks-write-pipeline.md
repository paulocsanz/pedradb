# RFC-0055: pipeline de escrita Rocks — medir antes de implementar

**Status:** in-progress
**Updated:** 2026-08-23
**Parents:** [0045](0045-multi-writer-async-5x.md) (bypass vs merge; hold 1.6 µs),
[0050](0050-nine-axis-robustness.md) (P0.4 teto; P2.1 memtable fora do lock),
[0054](0054-close-official-gaps.md) (buracos oficiais são 1-cliente)

## Background

O drop-in aceita e **ignora** três knobs que o Rocks tem de verdade:

| Rocks | O que faz | Pedra hoje |
|---|---|---|
| `allow_concurrent_memtable_write` | insert na memtable em paralelo; só o WAL serializa | apply **no write lock** (RFC-0050 P0.4) |
| multi-flush (`max_write_buffer_number` > 2) | um imm flusha enquanto outro recebe puts | dual-mem: 1 active + 1 imm; flush I/O fora do lock |
| `enable_pipelined_write` | group A no WAL enquanto group B prepara | um líder por vez; lone path sem canal |

Isso **não ferrando o cartaz oficial** é um fato de forma, não de fé:

1. Pernas oficiais (ycsb / deps / kvrocks 1c) são **um escritor**. Concurrent
   memtable e pipelined write são no-ops no 1c — não há segundo writer para
   paralelizar. `multiwriter_probe` (RFC-0045): hold 1.6 µs = wal 0.8 + mem
   0.5 + publish 0.25 + prepare 0.13. O mem apply é **um terço do hold**, e
   o hold é ~1% do wait com 50 threads (wait 175 µs). O teto 50c é agendamento,
   não skiplist.
2. O bench oficial pina memtable **256 MiB** — **zero flush** na janela.
   Multi-flush não dispara. O default de produto 4 MiB *é* mais rápido no
   apply com drain do que 64 MiB (2251 vs 1228 qps, `apply_flush_probe`).
3. Merge async (`PEDRA_ASYNC_GROUP=1`) já foi falsificado: 44–106 k vs
   bypass 311–636 k. Não é o caminho.

O que falta, se faltar, é no **mc50 / raftlog_mc4 / apply_mc4** — colunas
multi-writer, não no cartaz 1c. RFC-0050 P2.1 já registrou "concurrent
memtable apply, teto +15%, não 5×". Este RFC é o banco de benches que
impede implementar skiplist no escuro.

## Problems This Solves

- **Problem:** "não temos o pipeline do Rocks" soa a buraco; sem número,
  vira um trimestre de skiplist que as formas oficiais nunca pagam.
- **Problem:** os três setters compilam e não medem nada — um host que
  seta `max_write_buffer_number=4` acha que comprou 4 buffers.
- **Problem:** RFC-0045 / 0050 P2.1 apontam a mesma alavanca sem um
  harness único que um reviewer rode.

## Proposed Solution

1. Um example `pipeline_gap` que imprime, no mesmo processo:
   - 1c / 12c / 50c async (bypass) — lock wait vs hold vs fases
   - apply 4 MiB vs 64 MiB vs no-flush
   - flush-durante-write (imm no caminho)
2. Só implementar concurrent-memtable / extra buffers / pipeline se o
   P0 mostrar que uma **forma oficial ou mc4/mc50** está bloqueada por
   isso, com o número na mesa.
3. Até lá os setters continuam aceitos e **documentados inertes**.

## Delivery slices (mandatory)

### P0 — benches que respondem a pergunta

- [x] **P0.1** `pipeline_gap` example: 1c/12c/50c async — status: `done`
      (1c 18k, 12c 37k, 50c 152k — 1c nunca dispara concurrent memtable)
- [x] **P0.2** apply 4 / 64 / none + drain — status: `done`
      (4 MiB 5.9k c/ 3 SST; 64/none 0 SST nesta janela)
- [x] **P0.3** Finding `findings/rfc0055-p0/` — status: `done`
      (oficial 1c não paga; P1 não obrigado pelo número)

### P1 — implementar só se P0 obrigar

- [ ] **P1.1** Memtable insert fora do write lock (RFC-0050 P2.1) **iff**
      P0.3 mostrar ≥15% do gap mc50/mc4 no mem-apply — status: `todo`
- [ ] **P1.2** `max_write_buffer_number` real (N imm) **iff** uma forma
      oficial com o default 4 MiB flushar na janela e perder — status: `todo`
- [ ] **P1.3** pipelined write (WAL overlap) **iff** 1c não, e mc4
      mostrar encode+wal serializado como o buraco — status: `todo`

### P2 — polish

- [ ] **P2.1** Setters deixam de ser no-op só para os knobs que P1 ligou;
      os outros continuam inertes e listados — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | pipeline_gap 1c/Nc | done | example + finding | 2026-08-23 |
| P0.2 | p0 | apply 4/64/none | done | example + finding | 2026-08-23 |
| P0.3 | p0 | finding + veredito | done | findings/rfc0055-p0 | 2026-08-23 |
| P1.1 | p1 | mem apply off-lock iff | todo | — | 2026-08-23 |
| P1.2 | p1 | N write buffers iff | todo | — | 2026-08-23 |
| P1.3 | p1 | pipelined write iff | todo | — | 2026-08-23 |
| P2.1 | p2 | setters deixam de mentir | todo | — | 2026-08-23 |

## Acceptance Criteria

- **Tests:** example compila; `cargo run -p rocksdb-parity-bench --example pipeline_gap --release` imprime as três tabelas.
- **Telemetry:** finding `findings/rfc0055-p0/` com 1c vs Nc e 4 vs 64. Sem claim de cartaz.
- **Documentation:** este RFC; `docs/rocksdb-compat.md` memtable 4/64/256.
- **Screenshots:** none — backend-only.

## Out of scope

- Fechar raftlog / mvcc / blob / scan (RFC-0054).
- Ingest, compaction filters, WBWI, CFs reais (RFC-0050 P1 / out of scope).
- Religar `PEDRA_ASYNC_GROUP` como default (já falsificado).
- Mudar o default 4 MiB do produto sem número novo.
