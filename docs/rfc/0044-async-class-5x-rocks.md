# RFC-0044: ≥ **5×** RocksDB **async** na mesma classe (não é o cartaz G1)

**Status:** in-progress
**Updated:** 2026-08-19
**Parents:** [0041](0041-2x-rocks-default.md) (cartaz = Pedra G1 vs Rocks `sync=false`),
[0043](0043-high-level-2x-expanding-benches.md) (catálogo que só cresce),
[AGENTS.md](../../AGENTS.md)

## Background

- O **cartaz de produto** não muda: Pedra `fdatasync` antes do Ok vs Rocks
  default `WriteOptions.sync=false`. 1-op put nessa coluna é teto `1/t_fd`.
- Pedido extra: na **mesma classe** (os dois sem fsync no Ok), cada shape
  async ≥ **5×** o Rocks async. Isso mede o LSM/CPU, não o syscall de
  durabilidade.
- Knob: `PEDRA_PARITY_ASYNC=1` no binário **compat** (e
  `set_write_sync(false)` nas suítes cujo host já é async: kvrocks, ycsb,
  deps, qs, nebula, …). Default de `OpenOptions.sync` continua `true`.
- Compare JSON: `compat.sync=false` + durability
  `async-wal (PEDRA_PARITY_ASYNC=1; … NOT G1, not official)`.
- Lab sujo
  ([rfc0043-pedra-async-dirty](../../findings/rfc0043-pedra-async-dirty/)):

| shape | x5b | kvrocks4 vs Rocks `kvrocks/` | ≥5×? |
|---|---:|---:|---|
| `kvrocks_pipelined_set` | 1.56 | **9.9** | **sim** |
| `kvrocks_blob_set` | 1.96 | **16.6** | **sim** |
| `kvrocks_scan` | **44** | **15.5** | **sim** |
| `kvrocks_set` 1c | 2.14 | **5.00** | **sim** |
| `kvrocks_set_mc50` | **5.07** | 4.76 | p0 done |
| `kvrocks_get` | 1.43 | 3.44 (p50 <50 ns) | não |
| ycsb E | **5.61** | 3.91 same-run / 5.9 vs x5b Rocks | harness mudou |
| ycsb A/B/C/D/F | 1.6–2.6 | A 1.84 C 1.69 F 1.85 same-run | P2.2 |

- Mecânica já na árvore: WAL stage 1 MiB quando `!need_sync`;
  `commit_async_ops` (sem mpsc/catch-up); compact worker **não** acorda
  por put; CF `default` raw se a suíte não precisa de CFs nomeadas;
  memtable do bench 256 MiB (sem flush na janela medida);
  WAL v2 omite payload internado repetido; `insert_many`;
  `get_probe` sem `to_vec`.

## Problems This Solves

- **Problem:** o 1-op put vs Rocks async misturava G1 com “o motor é
  lento”. Sem coluna same-class o buraco do `fdatasync` esconde o CPU.
- **Problem:** write-group + catch-up sem fsync só adiciona espera
  (`set_mc50` ~0.5–0.7). Rocks é N threads + um lock.
- **Problem:** pipeline já >1 mas ≪5× — 32× memcpy/BTree vs
  `WriteBatch` C++.
- **Problem:** sem RFC, `PEDRA_PARITY_ASYNC=1` parece default de produto.

## Como fechar 5× (mapa, não wish)

Piso = `Pedra_qps / Rocks_qps` na **mesma** run quieta, peer `sync=false`.
Não usar Rocks doente. Números abaixo = Pedra vs Rocks saudável (`kvrocks/` + x5b ycsb).

| shape | hoje (kvrocks3 vs Rocks saudável) | Pedra precisa | o que falta |
|---|---:|---|---|
| pipeline | **9.9–11.8×** | — | P1.1 fechou |
| blob | **16–21×** | — | P1.2 fechou |
| set 1c | **5.00×** vs Rocks `kvrocks/` | — | P1.2 fechou |
| scan | **15.5×** (kvrocks4) | — | stall 35 ms foi dirty |
| mc50 | 4.76–5.07 | quiet | P0 5.07 |
| get | 3.44× (p50 <50 ns) | wall=p50 | cauda; P1.3 |
| ycsb E | 3.91 same-run / 5.9 vs x5b Rocks | +28% same-run | Rocks também ganhou ytab |
| ycsb A/F | 1.84 / 1.85 same-run | put+get 1c | epoch bump mata TLS |
| ycsb B/C/D | 1.07 / 1.69 / 1.46 | get | Rocks `get_pinned` também subiu |

Não fecha (e não se mente):

- 5× GET **copiando** 1 KB para o cliente a 5.5 M qps (~5.5 GB/s) — o canary é lookup (`get_probe`), como Rocks `get_pinned`.
- 5× 1-op **G1** vs Rocks async — teto `1/t_fd` (RFC-0041).
- Arena / skip-list C++ — só se coalesce+harness saturarem BTree (out of scope).

## Proposed Solution

1. **Duas colunas, dois cartazes.** G1 vs Rocks default = RFC-0041.
   Async vs async = **este RFC**, piso **5.0**, nunca vendido como “batemos
   o Rocks.”
2. Async 1-op / batch: `commit_async_ops` (lock, encode, mem, stage WAL).
   Group commit fica para quem `do_sync=true`.
3. WAL async: um `write()` por ~1 MiB, não por SET (blocos físicos 32 KiB
   intactos). `close`/`sync` drenam.
4. Gate: `ROCKS_PARITY_RATIO_FLOOR=5` nas shapes da coluna async quando
   `PEDRA_PARITY_ASYNC=1`. Compare recusa misturar G1 com “5× oficial.”
5. O que já ≥5× **fica** no relatório; o que falta entra como `todo`,
   não some.

## Garantias invariáveis

| # | Garantia | Este RFC |
|---|---|---|
| G1 | `fdatasync` antes do Ok **no default** | intocada; só o knob de bench |
| G6 | sem thread **no core** | intocada (worker compact já é compat) |
| G8 | peer oficial = Rocks `sync=false` | o cartaz 0041; esta coluna é extra |
| — | shape existente não some | **0043** |

## Delivery slices (mandatory)

### P0 — coluna existe + mc50 ≥ 5× (já no código)

- [x] **P0.1** RFC + status viva (este doc) — status: `done`
- [x] **P0.2** `PEDRA_PARITY_ASYNC=1` + `set_write_sync` no compat;
      WAL `write_pending_frame_if`; teste
      `async_puts_stage_wal_until_close` — status: `done`
- [x] **P0.3** `commit_async_ops` (sem write-group no async 1-op/batch);
      compact não notifica por put — status: `done`
- [x] **P0.4** `kvrocks_set_mc50` ≥ 5.0 async/async (x5b **5.07**, sujo)
      — status: `done` (quiet 3× = P2.1)

### P1 — o resto do kvrocks ≥ 5×

- [x] **P1.1** `kvrocks_pipelined_set` ≥ 5.0 — status: `done`
      (kvrocks3 **10.5** this / **11.8** vs Rocks `kvrocks/`)
- [x] **P1.2** `kvrocks_set` / `kvrocks_blob_set` ≥ 5.0 — status: `done`
      (kvrocks4 set **5.00** vs Rocks `kvrocks/`; blob **16.6**)
- [ ] **P1.3** `kvrocks_get` ≥ 5.0 — status: `doing`
      (3.44× vs saudável; p50 <50 ns, wall=cauda)

### P2 — YCSB + quiet 3×

- [ ] **P2.1** Remesura 3× quieta `findings/rfc0044-p2/`; não gravar
      mediana suja como oficial 0041 — status: `todo`
- [ ] **P2.2** ycsb A–F ≥ 5.0 na coluna async — status: `doing`
      (`get_probe`+ytab; same-run A 1.84 C 1.69 E 3.91 F 1.85)
- [x] **P2.3** Script: `PEDRA_PARITY_ASYNC=1` + `FLOOR=5` **não** é o
      default do `tikv_ycsb_parity_v0.sh` — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status viva | done | este doc | 2026-08-19 |
| P0.2 | p0 | knob async + WAL stage | done | `write_pending_frame_if` | 2026-08-19 |
| P0.3 | p0 | `commit_async_ops` | done | sem group no async | 2026-08-19 |
| P0.4 | p0 | set_mc50 ≥ 5× async | done | x5b 5.07 sujo | 2026-08-19 |
| P1.1 | p1 | pipeline ≥ 5× | done | kvrocks3 10.5 / 11.8 saudável | 2026-08-19 |
| P1.2 | p1 | set / blob ≥ 5× | done | kvrocks4 set 5.00 / blob 16.6 | 2026-08-19 |
| P1.3 | p1 | get ≥ 5× | doing | 3.44×; p50 <50 ns | 2026-08-19 |
| P2.1 | p2 | quiet 3× | todo | findings/rfc0044-p2 | 2026-08-19 |
| P2.2 | p2 | ycsb A–F ≥ 5× async | doing | get_probe+ytab; E 3.91 same-run | 2026-08-19 |
| P2.3 | p2 | script não default 5× | done | tikv_ycsb_parity_v0.sh | 2026-08-19 |

## Acceptance Criteria

- **Tests:** `async_puts_stage_wal_until_close`;
  `interned_pipeline_batch_recovers_after_close`;
  `v2_reuses_interned_value_bytes`;
  `fragment_encoded_matches_scratch_path_bytes` (inclui interned);
  crash-after-sync-put (G1) continua a recuperar;
  `kvrocks_suite_on_compat_engine`;
  `put_batch_same_interns_and_reads_back`.
- **Telemetry:** `findings/rfc0043-pedra-async-dirty/` (sujo) e, no P2,
  `findings/rfc0044-p2/` com `compare_report.json` ×3,
  `compat.sync=false`, `rocks.sync=false`.
- **Documentation:** este RFC; README do finding diz **not official**.
- **Screenshots:** none — backend-only.

## Out of scope

- Vender ratio async/async como “batemos o Rocks.”
- 5× no 1-op **G1** vs Rocks async (teto `1/t_fd`; RFC-0041 P1.2).
- Trocar o peer oficial para `sync=true`.
- Arena/skip-list C++ (follow-up se P1.1–P1.3 saturarem memcpy+BTree).
