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

Contrato async = encode + `write()` aos **64 KiB** (Rocks file writer),
sem `fdatasync`. Lab sujo `findings/rfc0044-p1/kvrocks-64k/` + `ycsb-64k/`.
Acked em userspace 1 MiB = **inválido**. `write()` em todo o put era mais
estrito que o Rocks e empatava o qps.

| shape | Pedra | Rocks | ratio | ≥5×? |
|---|---:|---:|---:|:---:|
| `kvrocks_scan` | 1.02 M | 20 k | **50.0** | **sim** |
| `kvrocks_pipelined_set` | 131 k | 12 k | **11.3** | **sim** |
| `kvrocks_get` | 3.53 M | 0.89 M | 3.97 | não (peer GET baixo) |
| `kvrocks_set` | 717 k | 186 k | 3.86 | não |
| `ycsb_e` | 346 k | 89 k | 3.89 | não |
| `kvrocks_set_mc50` | 200 k | 59 k | 3.38 | não |
| `ycsb_a` | 757 k | 347 k | 2.18 | não |
| `ycsb_d` | 1.61 M | 840 k | 1.92 | não |
| `ycsb_c` | 2.75 M | 1.48 M | 1.86 | não |
| `kvrocks_blob_set` | 50 k | 31 k | 1.62 | não |
| `ycsb_f` | 295 k | 239 k | 1.24 | não |
| `ycsb_b` | 1.23 M | 1.01 M | 1.21 | não |

Melhor janela da sessão (load ~14, `kvrocks-l14/`): SET **5.66**,
mc50 **9.24** (peer doente), scan 14.0, GET 4.22, blob 3.03,
pipeline **0.96** (cauda de `write()` no disco sujo; p50 segue 5× melhor).

- Mecânica já na árvore: `commit_async_ops` faz **`write()` antes do Ok**,
  sem `fdatasync` (não acked em userspace); compact worker **não** acorda
  por put; CF `default` raw se a suíte não precisa de CFs nomeadas;
  memtable do bench 256 MiB (sem flush na janela medida);
  WAL v2 omite payload internado repetido *dentro do mesmo batch*;
  `insert_many`; `get_probe` sem `to_vec`.

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
| scan | **50×** | — | KeyOnly + TLS; fecha |
| pipeline | **11.3×** | — | intern+v2+64 KiB; fecha |
| SET 1c | **3.86×** | +30% | same-run sujo `kvrocks-merge/` **6.91** (1.47 M vs 212 k) — arbitro é a quieta 3× (P2.1) |
| mc50 | **3.38×** | +48% | merge de líder testado: **0.19× vs bypass** (A/B 5 rounds, `kvrocks-merge/`); handoff de lock é o teto |
| blob | **1.62×** | 4×16 KB no buffer | same-run `kvrocks-merge/` 2.30 — copies de 16 KB dominam |
| A / F | 2.18 / 1.24 | put+get | F ainda RMW |

Não fecha (e não se mente):

- 5× GET **copiando** 1 KB para o cliente a 5.5 M qps (~5.5 GB/s) — o canary é lookup (`get_probe`), como Rocks `get_pinned`.
- 5× 1-op **G1** vs Rocks async — teto `1/t_fd` (RFC-0041).
- Merge de escritores async num líder único — A/B pareado
  (`findings/rfc0044-p1/kvrocks-merge/`): **0.19× vs bypass**. Em 50
  threads / 12 CPUs o líder é ponto único de agendamento. O código fica
  atrás de `PEDRA_ASYNC_GROUP=1` (default off) para reteste em caixa
  quieta; o produto é o bypass (N threads + um write lock, formato Rocks).
- Arena / skip-list C++ — só se coalesce+harness saturarem BTree (out of scope).

## Proposed Solution

1. **Duas colunas, dois cartazes.** G1 vs Rocks default = RFC-0041.
   Async vs async = **este RFC**, piso **5.0**, nunca vendido como “batemos
   o Rocks.”
2. Async 1-op / batch: `commit_async_ops` (lock, encode, mem, stage WAL).
   Group commit fica para quem `do_sync=true`.
3. WAL async = Rocks `sync=false`: encode no frame, `write()` aos
   **64 KiB** (`ASYNC_WAL_BUFFER` = `writable_file_max_buffer_size`).
   Não 1 MiB. Não ops por encode. Tail &lt; 64 KiB até o próximo flush /
   close. Sem `fdatasync`. Blocos físicos continuam 32 KiB.
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
      `async_puts_are_in_wal_without_fsync` — status: `done`
- [x] **P0.3** `commit_async_ops` (sem write-group no async 1-op/batch);
      compact não notifica por put — status: `done`
- [x] **P0.4** buffer async 64 KiB (Rocks file writer), não 1 MiB —
      status: `done` (`ASYNC_WAL_BUFFER`; testes 64 KiB + tail no close)
- [ ] **P0.5** `kvrocks_set_mc50` ≥ 5.0 async/async — status: `doing`
      (**veredito quieto: 2.02** vs peer são 152 k — os 3.4–9.2
      anteriores eram Rocks doente sob carga (55 k); merge rejeitado
      0.19×; o gap é outro mecanismo, não handoff de grupo)

### P1 — o resto do kvrocks ≥ 5×

- [ ] **P1.1** `kvrocks_pipelined_set` ≥ 5.0 — status: `doing`
      (reaberto pelo árbitro quieto: **straddle 4.22–5.86** entre
      janelas — 11.3 @ 64 KiB e 5.86 @ 2M load ~100, mas 4.22 @ 2M
      quieto; p50 4.0 µs vs 23 µs segue ~6× melhor. Não é ≥5 estável)
- [ ] **P1.2** `kvrocks_set` / `kvrocks_blob_set` ≥ 5.0 — status: `doing`
      (SET **cruza na quieta longa: 5.41** @ 2M quieto, p50 0.4 µs vs
      2.5 µs; curta 4.61. Blob **2.02** quieto — longe; copies de 16 KB
      dominam)
- [ ] **P1.3** `kvrocks_get` ≥ 5.0 — status: `doing`
      (straddle: 20 M ops 4.4–5.5; 2 M load ~100 **5.50**; 2 M quieto
      **4.61** com Rocks são a 2.5 M — p50 0.0 µs vs 0.4 µs. Não é ≥5
      estável; janela curta 1.61)

### P2 — YCSB + quiet 3×

- [x] **P2.1** Remesura 3× quieta `findings/rfc0044-p2/` — status: `done`
      (**árbitro executado a load 9.4**: fecham ≥5 consistentes **E
      (10.5–12.8)** e **scan (7.4)**; SET cruza na longa quieta (5.41);
      GET/pipeline straddle 4.2–5.9; mc50/blob ~2.0; F 1.66; A–D
      2.9–3.8; `deps_lock_prewrite` 0.94. Piso ≥5 para todos **não**
      alcançado — registrado sem maquiagem em `rfc0044-p2/quiet/`)
- [ ] **P2.2** ycsb A–F ≥ 5.0 na coluna async — status: `doing`
      (quieto 3×: **E fecha 10.5 med (3/3 ≥5 em toda condição) — P2.2
      parcial E fechado**; A 2.88 B 2.93 C 3.84 D 3.02 F 1.66 não
      fecham no wall (p50/p99 do F 1.5×/22× melhores). Cliff do E no 2M
      mapeado e fixado no produto (CountCache); `auto_reclaim` opt-in.
      **Nota (2026-08-21, RFC-0046 P0): o cliff do E também fecha
      estruturalmente por retention — o default do kernel agora é
      `Window(24 h)` bounded + archive (versões velhas saem do SSD,
      scan frio volta a O(live set + janela)), não só via CountCache
      no caminho quente. Re-árbitro oficial: RFC-0046 P0.4.**
      `ycsb-longwindow/` + `rfc0044-p2/quiet/`)
- [x] **P2.3** Script: `PEDRA_PARITY_ASYNC=1` + `FLOOR=5` **não** é o
      default do `tikv_ycsb_parity_v0.sh` — status: `done`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + status viva | done | este doc | 2026-08-19 |
| P0.2 | p0 | knob async; write() no Ok | done | sem userspace ack | 2026-08-19 |
| P0.3 | p0 | `commit_async_ops` | done | sem group no async | 2026-08-19 |
| P0.4 | p0 | buffer async 64 KiB | done | `ASYNC_WAL_BUFFER` | 2026-08-19 |
| P0.5 | p0 | set_mc50 ≥ 5× async | doing | quieto: **2.02 vs peer são** — não fecha; 3.4–9.2 eram peer doente | 2026-08-20 |
| P1.1 | p1 | pipeline ≥ 5× | doing | reaberto: straddle 4.22 (quieto 2M) – 5.86; p50 ~6× | 2026-08-20 |
| P1.2 | p1 | set / blob ≥ 5× | doing | SET **5.41 quieto 2M** (cruza); blob 2.02 | 2026-08-20 |
| P1.3 | p1 | get ≥ 5× | doing | straddle 4.61 (quieto) – 5.50; p50 0.0 vs 0.4 µs | 2026-08-20 |
| P2.1 | p2 | quiet 3× | **done** | **árbitro @ load 9.4**: E 10.5 e scan 7.4 fecham; SET 5.41 longo; resto 0.94–3.8 | 2026-08-20 |
| P2.2 | p2 | ycsb A–F ≥ 5× async | doing | E fechado (10.5 quieto, 3/3 toda condição); A–F demais 1.7–3.8 no wall | 2026-08-20 |
| P2.3 | p2 | script não default 5× | done | tikv_ycsb_parity_v0.sh | 2026-08-19 |

## Acceptance Criteria

- **Tests:** `async_puts_are_in_wal_without_fsync`;
  `async_ok_write_wal_without_fsync_survives_reopen`;
  `async_tail_below_64kib_recovers_on_close_without_fsync`;
  `async_concurrent_writers_recover`;
  `async_and_sync_concurrent_writers_recover`;
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
- Buffer async 1 MiB / Ok sem encode (rejeitado: pior que Rocks).
