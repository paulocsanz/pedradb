# RFC-0059 — Substituição real, anti-overindex e gate Linux (x86-64)

Data: 2026-08-24 · Status: `in-progress` · Precede: [RFC-0054](0054-close-official-gaps.md) · Par: [RFC-0041](0041-*.md) (piso 2×)

## Motivo

Três perguntas abertas depois do RFC-0054:

1. **O scoreboard Apple Silicon representa Linux/x86-64?** Todo número
   oficial até agora é macOS/aarch64 (M-series, F_FULLFSYNC no peer
   real-Rocks). O produto roda em servidores Linux/x86-64 com `fdatasync` +
   io_uring — o peer pode ser mais lento no Mac do que será no Linux, ou o
   contrário. Sem bateria Linux, publicar é aposta.
   Correção honesta: a VM caixote disponível é **AMD Ryzen Threadripper
   PRO 3975WX** (kernel 6.12.94-0-virt, 4 vCPU, linux/amd64 — cpuinfo
   confirmado no serial). É silício de servidor x86-64 diferente do
   M-series, o que basta para a pergunta "o Mac está enviesando?"; **não**
   é Intel, então quem precisar de número Intel específico ainda precisa
   de uma bateria Intel (não-meta aqui).
2. **Estamos otimizando pro benchmark ou pro mundo?** Todo shape oficial é
   zipfiano com `records=1024` — working set inteiro quente em cache, sem
   uniformidade, sem conjunto maior. Um motor que vence só nesse regime é
   um motor de benchmark.
3. **"Paridade" medida no nosso harness é "paridade" num DB de verdade?**
   O `rocksdb-compat` cobre a superfície do harness. Um banco upper
   (SurrealDB `kv-rocksdb`) exercita a mesma API por outro caminho — e
   exigia `rocksdb = "0.21.0"` por nome de crate.

## P0 — bateria Linux/x86-64 (gate de publicação)

- [x] **P0.1** Bateria oficial (3 rounds × 4 pernas, ops=2000, mesmo
      script POSIX com gate de load) em VM linux/amd64 4 vCPU
      (caixote, região brasil), fonte `9edc373` empacotada na imagem
      (`findings/2026-08-24-linux-battery1/`).
- [ ] **P0.2** Scoreboard Linux documentado neste RFC com coluna própria —
      números pendentes da primeira bateria.
- [ ] **P0.3** `scripts/parity_gate_closed.py` verde sobre a bateria Linux
      (mesmas regras da rearm11; se falhar, é bug de portabilidade, não
      de meta).

## P1 — anti-overindex (benchmark ≠ mundo)

- [x] **P1.1** Pernas uniformes: `ycsb_b_unif` (95% read, sem zipf) e
      `ycsb_c_unif` (100% read uniforme) no bin oficial — mesma mistura,
      distribuição trocada (`run_dist`).
- [x] **P1.2** Working set grande: `ycsb_c_big` — 2^20 chaves semeadas
      sem tempo medido, depois gets uniformes (`run_c_big`, knob
      `ROCKS_PARITY_BIG=0` desliga). Nada de hot-set em cache.
- [ ] **P1.3** Ratios das novas pernas no Linux + leitura: se o ratio cair
      >30% do shape zipfiano correspondente, o motor está overindexado e
      vira bug de performance geral (não de shape).

## P2 — substituição real (upper DB no lugar do RocksDB)

- [x] **P2.1** Shim crate `crates/rocksdb` (package `rocksdb` v0.21.0,
      features `lz4`/`snappy` no-op) reexportando `rocksdb-compat` +
      alias `Transaction<'a, D>` parametrizada no DB — SurrealDB v1.5.4
      compila sem tocar uma linha dele via `[patch.crates-io]`.
- [x] **P2.2** Bench de substituição (`sub-bench`): SurrealDB
      `Datastore`/`Transaction` idêntico dos dois lados, storage trocada
      só pelo patch. Pernas: `point_read`, `point_write`, `scan`
      (kvs `scan` = raw_iterator + seek), `doc_txn` (RMW com validação
      OCC). Sem meta de ratio — é prova de substituição correta + telemetria
      de onde o custo upper mora.
- [ ] **P2.3** Rodar sub-bench no Linux (mesma VM) e arquivar JSON.

## Não-metas

- `deps_scan`/`kvrocks_blob_set`/`kvrocks_set_mc50` ≥2× — explicitamente
  depriorizados pelo usuário (2026-08-24); seguem abertos no RFC-0054.
- v2.x do SurrealDB (SstFileManager/Env/disk_space_manager) — a superfície
  dobrou; a substitution provada na v1.5.4 basta para este RFC.

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | bateria Linux oficial | done | findings/2026-08-24-linux-battery1 (VM bench-4) | 2026-08-24 |
| P0.2 | p0 | scoreboard Linux | pending | primeira bateria em curso | 2026-08-24 |
| P0.3 | p0 | gate verde no Linux | pending | idem | 2026-08-24 |
| P1.1 | p1 | ycsb_b_unif + ycsb_c_unif | done | run_dist no bin oficial | 2026-08-24 |
| P1.2 | p1 | ycsb_c_big 2^20 | done | run_c_big, knob ROCKS_PARITY_BIG | 2026-08-24 |
| P1.3 | p1 | ratios anti-overindex Linux | pending | precisa bateria com novo bin | 2026-08-24 |
| P2.1 | p2 | shim rocksdb 0.21 | done | crates/rocksdb + alias Transaction | 2026-08-24 |
| P2.2 | p2 | sub-bench SurrealDB | done | bench/sub-surreal no repo; kvs::tests 62/62 no shim (pós-F183) | 2026-08-24 |
| P2.3 | p2 | sub-bench no Linux | pending | — | 2026-08-24 |

## Acceptance Criteria

1. Bateria Linux completa arquivada com JSONs crus + tabela de ratios.
2. `parity_gate_closed.py` exit 0 sobre a bateria Linux.
3. Novas pernas anti-overindex com ratio reportado ao lado do zipfiano.
4. `surrealdb-core` v1.5.4 compilando e passando smoke de leitura-escrita
   sobre o shim sem `unsafe` novo no lado Pedra.
