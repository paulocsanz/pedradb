# RFC-0238 — Fechar as fatias EG1 pagáveis neste ambiente: `ycsb_b_mc4` no cartaz Linux + ladder fjall in-tree

**Status:** in-progress (P0.1 done; P0.2 done; P1.1 done; P1.2 measured —
não pagou, segue aberto; P1.3/P2.1 todo)
**Updated:** 2026-09-21
**ID:** 0238
**Parents:** [TRAJETORIA](../TRAJETORIA.md)
(EG1 — velocidade; escada A9/C1/C2/C3 pagáveis sem caixa dedicada; gate
sequencial: EG1 é a frente ativa),
[0185](0185-coluna-a-dropin-1x-tudo.md)
(P2.2: todo `COMPARE_SHAPES` fora de G_A — Linux 3-run ≥1.0 ou C;
família Darwin ≪1× primeiro, `ycsb_b_mc4` nomeado; "harness stall
conserta-se no runner"),
[0233](0233-ganhar-sempre-rocks-fjall-default.md)
(fjall como par absoluto — nunca `compat_over_rocksdb`),
[0235](0235-classe-workload-empatar-rocks-fjall-todas-escalas.md),
[0237](0237-async-vs-async-paridade-wal-miss.md)
(ladder fjall foi harness efêmero fora da árvore; bug SIGSEGV P0.1 é
fronteira nomeada de A4, não deste RFC)
**Peer Rocks:** `WriteOptions.sync=false` (`ROCKS_PARITY_SYNC=0`), JSON
`sync:false` do peer no finding. Darwin = DIAG. Linux 3-run quiet = cartaz.
**Peer Fjall:** QPS **absoluto** no mesmo host/binário/protocolo. Nunca
`compat_over_rocksdb`.

> **Tese:** o EG1 tem 13 fatias e precisa de ≥50% (`floor(100×(done+½·doing)/13)`).
> As células engine-deep de menor ratio (probe_miss 0.29×, overwrite_mc4
> 0.557×) são caixa-pinned ou dataset-pinned (33 GiB) e não fecham neste
> ambiente; o conjunto pagável-sem-caixa (A9 + C1 + C2 + C3) soma 4 fatias
> — 7,5/13 ≈ 57% — e cada uma tem produto real: (1) `ycsb_b_mc4` nem
> existe no harness multi-cliente (o array `shapes` de `run_clients` só
> emite `ycsb_a`/`ycsb_f`/`deps_cache_overwrite`); (2) o ladder fjall foi
> scratch fora da árvore (RFC-0182: sem crate novo — estende o
> `rocksdb-parity-bench` com feature `fjall`). Paga = Linux 3-run quiet
> ≥ bar (Rocks ≥1.0 / fjall absoluto ≥), nunca Darwin, nunca peer
> colapsado, nunca `sync:true`.

## Rank (impacto × pagável-agora)

| # | fatia TRAJETORIA | por que agora |
|---|---|---|
| 1 | A9 `ycsb_b_mc4` | célula BALANCE sem número Linux (único G_A-adjacente sem nem medição); 1 linha de harness + 3-run |
| 2 | C1/C2 fjall seq 64k/1M | 2 fatias; Darwin DIAG 1.163×/1.614× diz que o motor está na frente; falta o número Linux in-tree |
| 3 | C3 expansão ladder | 1 fatia; shapes novos cozinhados no harness oficial (append-only) |
| — | A4/A5/A6/A7/A8 | caixa/dataset-pinned (4 GiB SKU, 25M/100M) + engine-deep (bug SIGSEGV 0237 + alavanca lock_wait) — **fora deste RFC**, próximos cortes nomeados |

## Delivery slices (mandatory)

### P0 — as fatias pagáveis

- [ ] **P0.1** `ycsb_b_mc4` no harness + Linux 3-run ≥1.0 quiet vs Rocks
  default. O array `shapes` de `run_clients` ganha `("ycsb_b", 95, false,
  false)` (mesmo mix do 1c, espelhando ycsb_a/ycsb_f). Teste nomeado:
  `run_clients_only` com `only=ycsb_b_mc4` dirigindo `CompatEngine` real.
  — status: `in-flight`
- [ ] **P0.2** fjall in-tree (RFC-0182: estende `rocksdb-parity-bench`):
  feature `fjall`, `FjallEngine` implementando `Engine` (put/get/
  get_probe/scan_count/flush), suíte `ladder` com `fjall_seq_64k` /
  `fjall_seq_1m` (put sequencial em ordem de chave, working set 64k/1M,
  payload do `Cfg`, `LADDER_OPS` ops medidos, xorshift `0x5EED_0001`).
  Teste nomeado: roundtrip put/get/scan via `FjallEngine` + emissão da
  shape. — status: `in-flight`

### P1 — expansão e profundidade

- [ ] **P1.1** C3: shapes de expansão do ladder (`fjall_rand_rw_1m`
  50/50 put/get random sobre 1M chaves; `fjall_scan_1k` scan_count em
  janelas de 1k) + Linux 3-run absoluto. — status: `todo`
- [ ] **P1.2** C1+C2: ladder Linux 3-run absoluto `compat` vs `fjall`,
  mesmo binário/host/protocolo, quiet. Paga QPS absoluto compat ≥ fjall
  por shape. — status: `todo`
- [ ] **P1.3** Bug SIGSEGV rollover do 0237 P0.1 (`mapped_pwrite` em
  mapping pós-rollover; job carrega o gen do segmento e revalida) +
  teste nomeado de regressão. Toca `*_kernel.rs` ⇒ ratchet same-fire
  completo (catálogo + Aeneas + Lean + gates). Desbloqueia A4.
  — status: `todo`

### P2 — engine-deep (caixa-pinned, fora do escopo 50%)

- [ ] **P2.1** A4 `overwrite_mc4` 25M @4 GiB ≥1.0 3-run (pós-P1.3 +
  alavanca lock_wait/grupo). — status: `todo`

## Protocolo do meter (por célula)

- **A9 (Rocks):** binário estático musl x86_64 (zigbuild, `--features
  real`) no gate Linux 4 vCPU/4 GiB. `ROCKS_PARITY_SYNC=0
  ROCKS_PARITY_CLIENTS=4 ROCKS_PARITY_SUITE=ycsb
  ROCKS_PARITY_ONLY=ycsb_b_mc4` (+`PEDRA_PARITY_ASYNC=1` no compat),
  engines alternados por round, `rm -rf` do dbdir entre shapes,
  quiet-gate `load1 < 2` ×2 antes de cada round (padrão p149). 3 rounds;
  paga mediana ≥1.0 quiet com peer JSON `sync:false` e Rocks não
  colapsado (referência do box registrada).
- **C1/C2/C3 (fjall):** mesmo binário (`--features real,fjall`), engines
  `compat` e `fjall` alternados, mesmo `Cfg` (`LADDER_OPS`, payload),
  3 rounds quiet. Paga QPS absoluto compat ≥ fjall em cada shape
  (mediana 3-run). Nunca vira `compat_over_rocksdb`.
- **Honestidade:** Darwin roda só como DIAG de sanidade do harness
  (corroboração de saúde, nunca cartaz); cada finding carrega host,
  kernel, load1 por round, e o rótulo quiet/not-quiet.

## Status (living — update with every PR)

| slice | status | evidência |
|---|---|---|
| P0.1 ycsb_b_mc4 harness + Linux 3-run | **done** | `findings/2026-09-21-rfc0238-a9-ycsb-b-mc4-linux/` — 3/3 quiet, peer `sync:false`, min 1.0010 / med 1.1268; canary floor calibrado (desvio documentado no finding) |
| P0.2 fjall engine + ladder in-tree | **done** | `FjallEngine` + suite `ladder` in-tree, named tests `rfc0238_*`; paga evidenciada pelo C3 |
| P1.1 C3 expansão ladder | **done** | `findings/2026-09-21-rfc0238-c3-fjall-shapes-linux/` — rand_rw med 1.0302 (min 1.0233); scan med 1.0872 (min 0.9959); errors 0 |
| P1.2 C1+C2 ladder Linux absoluto | **measured — não pagou** | `findings/2026-09-21-rfc0238-c1-c2-fjall-seq-linux/` — seq_64k med 0.4361×, seq_1m med 0.5133×; G1≈async no box ⇒ gargalo CPU-side Linux-specific; segue aberto |
| P1.3 SIGSEGV rollover (A4-unblock) | todo | — |
| P2.1 A4 overwrite_mc4 @4 GiB | todo | — |

## Não

- Darwin como cartaz; peer `sync:true`; Rocks colapsado como win.
- Fjall como `compat_over_rocksdb`; apagar/renomear shape para subir
  ratio (COMPARE_SHAPES append-only).
- Crate novo de bench (RFC-0182); `db_bench` C++.
- Marcar fatia sem finding datado com JSON do peer e `number: ratio=`
  (Rocks) ou QPS absoluto dos dois lados (fjall).
