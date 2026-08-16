# RFC-0035: MVCC latest + deps_scan to ≤2× vs Rocks FF — measure first

**Status:** in-progress  
**Updated:** 2026-08-16  
**Parents:** [0033](0033-mvcc-scan-2x.md) (last_under_prefix, lazy scan), [0034](0034-rocks-parity-1.1x-all-shapes.md) (teto 1.1× all-shapes; estes dois ainda longe)

## Background

- Alvo **destes dois shapes**: Pedra no máximo **2× mais lenta** que Rocks na mesma classe (`ROCKS_PARITY_FULL_SYNC=1`). Floor = `0.5 × peer FF da run`.
- Lab 2026-08-16 (`586ec6e`, 4096/2000 zipfian 1 KB, deps-only, depois do apply):

| shape | Pedra | p50 | Rocks FF | p50 | slower | floor 2× (qps) |
|---|---:|---:|---:|---:|---:|---:|
| `deps_mvcc_latest` | 9 006 | 77 µs | 159 790 | 5.4 µs | **18×** | ~80k |
| `deps_scan` | 6 117 | 162 µs | 164 889 | 5.7 µs | **27×** | ~82k |

- Já tentámos e **medimos** o que não chega: iterator em janela, `last_under_prefix`, lazy block + `BlockCache`, não clonar o bloco, seek `partition_point` no prefixo, `last_under_user_prefix` (salta SST se a mem já tem a latest). MVCC veio de 69× → 18×; scan de 35× → 27×. O 2× **não** é um patch óbvio que ficou por aplicar.
- Hipóteses **não fechadas** (tom de hipótese até haver número):
  1. Quantos SST/L0 cada latest/scan toca depois de 64k apply (p95 313 µs no MVCC cheira a caminho SST).
  2. `latest_cf` + `get_cf` = dois `Mutex<Db>` e dois walks do LSM.
  3. Get do valor 1 KB no `default` (vlog vs inline) no mesmo op do latest.
  4. Scan: merge de N streams mesmo com `limit=25` e `KeyOnly`.
  5. Custo por arquivo (índice, bloom, `get_or_insert` do cache) vs custo por key.
- Sem essa conta, um redesign (SeekForPrev de um bloco, bloom de prefixo, menos L0) é chute. Com a conta, ou o P1 é um patch pequeno ou o P2 é um redesign justificado.

## Problems This Solves

- **Problem:** 18× / 27× vs FF nestes dois shapes; o teto 2× do 0033 ficou aberto.
- **Problem:** os cortes já feitos melhoraram o número sem dizer *onde* estão os µs que faltam para 2×.
- **Problem:** patch vs redesign sem telemetria é teatro — ou afrouxamos garantia à toa, ou rodamos em círculo.

## Proposed Solution

1. **P0 = oráculo de custo, não de teoria.** Contadores e um split de tempo no próprio harness deps (mesmo schedule, FULL_SYNC). Relatório ranqueado: o que come p50 vs p95. Sem este relatório não se começa P1.
2. **P1 = o gargalo #1 que o P0 apontar**, um de cada vez, re-medir. Candidatos só entram se o P0 os tiver numerado. Visibilidade = `lookup` / `range_at`. Sem cap por camada. Sem `sync_data`.
3. **P2 = redesign só se o P1 esgotar o #1/#2 e ainda > 2×.** O follow-up cita o cliff medido (ex.: “4 L0 × 40 µs irredutíveis”). Não se inventa API nova para fugir do número.
4. **G1–G8 intactos.** Adversarial sem editar asserção. Gate 0.5 nestes dois shapes só quando a remesura da run passar.

## Garantias invariáveis

Herdadas de RFC-0031. Em especial:

| # | Garantia | Este RFC |
|---|---|---|
| G1 | WAL `sync_all` antes do Ok | read-path + contadores; **não** troca por `sync_data` |
| G2 | visibilidade get/scan/latest = `lookup` / `range_at` | sem cap por camada que esconda chave viva |
| G4 | adversarial compat | re-verde **sem** mudar asserção |
| G6 | sem thread no core | telemetria é inline / no bench, sem worker |
| G8 | números honestos | floor = 0.5 × **peer FF da mesma run**; não cravar 80k para sempre |

Editar asserção existente para ficar verde é relaxação.

## Delivery slices (mandatory)

### P0 — must ship first (útil sozinho: sabemos o quê atacar)

- [x] **P0.1** RFC + Status vivo (este doc) — status: `done`
- [x] **P0.2** Contadores no read-path (ou no harness deps): SST tocados, blocos decodificados vs cache hit, mem-hit vs fallback SST no latest, `sst_count` / L0 / L1 no momento do op — status: `done`
- [x] **P0.3** Split de tempo no op deps (`last_under_*` vs `get` default vs merge/scan emit), p50/p95, sem mudar o schedule — status: `done`
- [x] **P0.4** Finding durável: ranking dos gargalos com número; escolhe o alvo do P1.1 — status: `done`

### P1 — um gargalo de cada vez (só o que o P0.4 ranqueou)

- [x] **P1.1** Cortar `get_cf` 1 KB do default no path MVCC (37.5 µs p50; sozinho impede o 2×); remesura deps vs FF — status: `done` (mem-hit skip SST + um lock; **get é inline não vlog**; p50 get ~36 µs, 2× **não** atingido)
- [x] **P1.2** Cortar os 36 µs do get na mem; remesura — status: `done` (MemTable por user-key + parking_lot + encode stack; p50 MVCC **2.0 µs**; 2× qps **não**)
- [ ] **P1.3** `deps_mvcc_latest` e `deps_scan` ≥ 0.5 × Rocks FF da run — status: `todo` (scan emite 1 versão/user/camada; p50 117 µs — ainda > 2×)

### P2 — redesign só com cliff medido

- [ ] **P2.1** Se P1 esgotar e ainda > 2×: doc do cliff (µs irredutíveis no desenho atual) + uma proposta de redesign que **preserve G1–G8** — status: `todo`
- [ ] **P2.2** Gate `ROCKS_PARITY_RATIO_FLOOR=0.5` + `ROCKS_PARITY_GATE_SHAPES=deps_mvcc_latest,deps_scan` no `tikv_ycsb_parity_v0.sh` com FULL_SYNC=1 — status: `todo`

## Status (living — update with every PR)

| ID | Band | Title | Status | Task / PR | Updated |
|----|------|-------|--------|-----------|---------|
| P0.1 | p0 | RFC + medida-primeiro | done | este doc | 2026-08-16 |
| P0.2 | p0 | contadores SST/cache/mem-hit | done | `ReadProbeSnap` + JSON no bench | 2026-08-16 |
| P0.3 | p0 | split de tempo no op deps | done | `deps_mvcc_latest_split` latest vs get | 2026-08-16 |
| P0.4 | p0 | finding ranqueado → alvo P1 | done | [rfc0035-p0-ranked-bottlenecks.md](../findings/rfc0035-p0-ranked-bottlenecks.md) | 2026-08-16 |
| P1.1 | p1 | get_cf 1 KB no path MVCC | done | mem-hit skip SST + 1 lock; get 100% inline; p50 ~36 µs; 2× não | 2026-08-16 |
| P1.2 | p1 | cortar 36 µs do get na mem | done | borrowed mem get; p50 2.0 µs; qps 18.8k vs 252k FF = 13× | 2026-08-16 |
| P1.3 | p1 | MVCC+scan ≥ 0.5 vs FF | todo | 51k vs 228k FF = 4.4×; scan 8.0k / 117 µs = 29× | 2026-08-16 |
| P2.1 | p2 | redesign só com cliff medido | todo | — | 2026-08-16 |
| P2.2 | p2 | gate 0.5 nestes dois shapes | todo | — | 2026-08-16 |

## Acceptance Criteria

- **Tests:** adversarial `cargo test -p rocksdb-compat` **sem** editar asserção; `last_under_user_prefix` (mem hit não decodifica SST; tombstone da latest ainda vê a versão flushed); `try_scan_at` limit + tombstone inalterados. P0.2/P0.3: teste de que os contadores sobem num scan/latest conhecido (não “sempre zero”).
- **Telemetry:** `scripts/tikv_ycsb_parity_v0.sh` + `ROCKS_PARITY_FULL_SYNC=1` + suíte `deps`; JSON do bench inclui (P0) os contadores/splits; (P1.3) `meets_floor=true` em `deps_mvcc_latest` e `deps_scan` com floor 0.5. Finding em `findings/` no mesmo commit do P0.4.
- **Documentation:** este RFC + linha em `open-items.md`. P1 atualiza a tabela do 0033/0034 com a run nova.
- **Screenshots:** backend-only.

## Out of scope

- 2× (ou 1.1×) de **escrita** vs fdatasync (G1).
- Teto 1.1× nos 11 shapes (RFC-0034) — este RFC é só MVCC + `deps_scan`.
- Trocar `File::sync_all` por `sync_data`.
- Thread de compact / `engine_pedra` / cluster TiKV.
- Começar P1 sem o finding P0.4.
