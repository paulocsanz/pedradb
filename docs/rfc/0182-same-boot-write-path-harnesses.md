# RFC-0182 — Same-boot contra overfit do write path

**Status:** in-progress
**Updated:** 2026-09-07
**ID:** 0182
**Parents:** [0180](0180-overwrite-mc4-gt1x.md),
[0163](0163-anti-overfit-benchmark-breadth.md),
[0178](0178-vitoria-celulas-restantes.md)
**Peer:** RocksDB default `sync=false`. Fjall = 3º peer, absoluto.
G1 não é win.

> P0 é Darwin, harnesses **já existentes**. 25M/100M / 4 GiB é P1 —
> não assar sem pedido. Não WARM 100M em 4 GiB.

## Background

- RFC-0180 fechou `deps_cache_overwrite_mc4` Darwin 3-run mediana
  **1,002×** (p42; named loss 0,816; Rocks quiet 261–275 k). O shape
  é 1024 keys / 100 k ops / 4 clientes.
- Audit do `b35d77a`: TLS write-through saiu, histerese 200 µs
  skipava L0 a 5–10 kQPS, catch-up `active≤8`, flush cada 32 seqs.
  P0.34–P0.38 no `b944113` reverteram isso.
- DIAG p48 (1-run Darwin, `sync: false`), **limites do gain**:

  | shape | ratio | nota |
  |---|---:|---|
  | `ycsb_a_mc4` | **0,91** | p50 8,7 vs 3,2 µs (misto) |
  | `ycsb_f_mc4` | **0,74** | RMW; p50 9,2 vs 3,8 |
  | `deps_apply_batch_mc4` | **0,47** | Pedra max 7 s; serial apply |
  | `deps_cache_overwrite` 1c | **0,82** | chão conhecido |
  | `kvrocks_set_mc50` | **0,37** | Adaptive **off** a n=50; avg_group 1,0 |

- O repo **já tem** Fjall e snapshot-bench. Não há 4º harness a
  inventar. `rocks-parity-bench --features fjall` (YCSB-only).
  `crates/snapshot-bench` Pedra / Rocks 0.50 / Fjall 3 (sorted-ingest).
  `pedra scale` / `reproduce-scale.sh`. Fjall nunca é gate.

## Problems This Solves

- **Problem:** um PR de write path só corre overwrite_mc4 1024 keys.
- **Problem:** Fjall e snapshot-bench existem e o 0180 não os tocou.
- **Problem:** mediana 1,002 com 1/3 quieto a 0,816 passou a P0.9.

## Proposed Solution

Contrato same-boot **com bins que já compilam**. Depois de cada mudança
no write path: Pedra + Rocks `SYNC=0` nas células da tabela; Fjall
absoluto na mesma YCSB overwrite (não ratio). Snapshot 1M é o mínimo
de escala (não 100M Darwin). Quiet-host: Rocks overwrite_mc4 ≳260 k;
**3/3** quietos todos ≥1,0, não só a mediana.

## Delivery slices (mandatory)

### P0 — Darwin, receita + um same-boot

- [x] **P0.1** Este RFC — status: `done`
- [x] **P0.2** Receita no `docs/benchmarks.md`: overwrite_mc4 +
      ycsb_a_mc4 + ycsb_f_mc4 + apply_mc4 + 1c overwrite +
      `engine=fjall` overwrite (absoluto) + snapshot-bench 1M
      (`SLIPSTREAM_BENCH_BACKENDS` um de cada vez) — status: `done`
- [x] **P0.3** Um same-boot Darwin HEAD (`b944113`+): JSON + compare
      `sync: false`; Fjall qps absoluto; snapshot 1M get_hit/prefix.
      Named losses na tabela. Não é 3-run. — status: `done`

Darwin 1-run (`sync: false`; Rocks overwrite **161 k** ≲ quiet 260 k
— **not a win** on that row):

| shape | Pedra | Rocks | ratio |
|---|---:|---:|---:|
| overwrite_mc4 | 210 k | 161 k | 1,304 (Rocks collapsed) |
| ycsb_a_mc4 | 351 k | 520 k | **0,676** |
| ycsb_f_mc4 | 299 k | 513 k | **0,582** |
| apply_mc4 | 4,92 k | 10,3 k | **0,478** |
| 1c overwrite | 284 k | 336 k | **0,845** |

Fjall `ycsb_a_mc4` **501 k** absoluto. snapshot 1M get_hit 1,38 vs
2,58 µs; prefix 117 vs 309 µs. avg_group overwrite **2,46**.

### P1 — 3-run quieto e caixa

- [ ] **P1.1** overwrite_mc4 3-run **3/3 quietos ≥1,0×** (não mediana
      com 0,816) — status: `todo`
- [ ] **P1.2** ycsb_a_mc4 e ycsb_f_mc4 same-boot 3-run vs Rocks
      default (números; ≥1× ou ceiling nomeado) — status: `todo`
- [ ] **P1.3** Caixa 4 GiB: overwrite isolado (0180 P1.1 / 0178 P1.3)
      — status: `todo`

### P2 — polish

- [x] **P2.1** snapshot-bench `get_hit` criterion median → `classify_get`
      vs RFC-0176 clock (best/happy/worst/as_is). Same line as scale
      P2.6: `diagnose get get_hit/<backend> … class=…`. Always, not
      COST_TRACE-gated. — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC | done | este ficheiro | 2026-09-07 |
| P0.2 | p0 | receita docs/benchmarks.md | done | §RFC-0182 | 2026-09-07 |
| P0.3 | p0 | same-boot Darwin 1-run | done | findings/2026-09-07-rfc0182-p03-same-boot | 2026-09-07 |
| P1.1 | p1 | overwrite 3/3 quiet ≥1× | todo | — | 2026-09-07 |
| P1.2 | p1 | ycsb_a/f 3-run | todo | — | 2026-09-07 |
| P1.3 | p1 | caixa overwrite | todo | 0180 P1.1 | 2026-09-07 |
| P2.1 | p2 | snapshot-bench get_hit classify_get | done | criterion median vs 0176; `rfc0182_p21_snapshot_get_hit_classifies_vs_0176` | 2026-09-07 |

## Acceptance Criteria

- **Tests:** nenhum binário novo. P0.2 é doc + comando que exit 0 no
  smoke 1M / ycsb 1 k ops. P2.1:
  `rfc0182_p21_snapshot_get_hit_classifies_vs_0176` (1M 1,38 µs ≠
  as-is walk); `rfc0182_p21_as_is_walk_is_not_best`.
- **Telemetry / Analytics:** JSON `rocks_parity_bench` +
  `compare_report`; snapshot criterion stderr; Fjall **sem**
  `compat_over_rocksdb` como win. snapshot-bench `get_hit` imprime
  `diagnose get get_hit/<backend> … class=…` (P2.1).
- **Documentation:** este RFC; `docs/benchmarks.md` P0.2.
- **Screenshots:** backend-only.

## Out of scope

- Novo harness. `db_bench` C++. redb/sled adapter.
- Fjall como gate. G1 win. Peer `sync=true`.
- WARM 100M na caixa 4 GiB. Prefix 0,70× (0178 P1.2). Apply skiplist
  (0183 / 0055).
