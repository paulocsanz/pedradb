# Mapa — forte / fraco / teto / buraco de ferramenta

**Updated:** 2026-09-07
**Peer:** Rocks default `sync=false`. Linux = cartaz. Darwin = DIAG.
**Fonte dos números:** RFCs citados, não esta página.

Refresh this file when a cell moves class. One row per cell.

| cell | class | host | lever (last diagnose) | source |
|---|---|---|---|---|
| ycsb A/B/C/F 1c async | **S** | Linux floor1x | — | RFC-0041 / `findings/rocks-parity-floor1x/` min 1.254 |
| overwrite 1c async | **S** | Linux 2.46× | — | floor1x |
| apply 1c async | **S** | Linux 2.58× | — | floor1x |
| ycsb_a_mc4 25M | **S** | Linux 2.26× 3/3 | — | RFC-0163 Grid B |
| apply_mc4 G1 quiet | **S** | Linux 2.79× | group commit (fd/grupo) | RFC-0041 head3 |
| get 50M/100M clock | **S** | calibrated | class best/happy vs 0176; scale `get_hit` `classify_get` (0184 P2.6) | RFC-0176 / 0184 |
| hydrate 25M/100M | **S** | Linux >1× | — | RFC-0162 |
| **overwrite_mc4 25M** | **U** | Linux 0.557× 3/3; Darwin 0180 ~1.00 mediana (named 0.816); **caixa pós-0180 none** | unknown on Linux; Darwin avg_group 2.46 | RFC-0178 P1.3 / 0180 P1.1 / 0184 P1.1 |
| ycsb_f_mc4 3/3 intra | **W** | Linux mediana 1.47; run2 0.766 | `get_path` (`read_pct=50`; mc WRITEPHASE) | RFC-0178 P1.4 / 0184 |
| prefix 100M 4 GiB | **W** | caixa 0.70× | bounded-cache scan; scale `classify_probes` vs \(P_{\mathrm{best}}\) (0184 P2.5) | RFC-0178 P1.2 / 0184 |
| probe_miss | **W** | 0.27× | miss path; scale `classify_probes` vs \(P_{\mathrm{best}}\) (0184 P2.4) | RFC-0178 P1.1 / 0167 / 0184 |
| 1c overwrite Darwin | **C**/DIAG | 0.845× | `wal_encode_or_write` despark=0 | RFC-0183 / 0184 |
| apply_mc4 Darwin async | **C**/DIAG | 0.478× (pré P0.5) | `flush_check`; 0184 P0.5 default-over parks whole mem O(1) | RFC-0183 / 0184 P0.5 |
| kvrocks_set_mc50 | **C** | 0.37× | `lock_convoy` Adaptive off n≥16; WRITEPHASE → `diagnose.lever` (0184 P2.3) | RFC-0178 / 0183 / 0184 |
| G1 1c write-per-op | **C** | fd-ceiling | one barrier/op | Agents.md / floor1x-g1 |
| ycsb_a/f_mc4 Darwin mixed | **T**→tool | 0.58–0.91 DIAG | `get_path` with `--read-pct 50`; `balance_admits=0` (DIAG); compare `diagnose.lever` (0184 P1.2) | RFC-0182 / 0184 |
| Linux async apply_mc4 | **U** | G1 2.79× exists; same-class mc4 not in floor1x 15 | — | RFC-0184 |
| 1B get @64 GiB | **S** (model) | happy ~61 µs; as-is 12.3 ms | `pedra diagnose get` | RFC-0176 |
| lookup_100 / get_loop | **T**→tool | Darwin 427 µs @100M | scale `classify_get` ns/100 vs 0176 (0184 P2.7) | RFC-0178 P0.9 / 0184 |
| qs_hot_get / qs_neg / qs_batch_write | **T**→tool | RFC-0043 HL | WRITEPHASE → `diagnose.lever` (0184 P2.8); hot/neg `get_path` | RFC-0043 / 0184 |
| kvrocks 1c get/set/scan | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.9); get/scan `get_path` | RFC-0043 / 0184 |
| myrocks + linkbench_mix | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.10); select/range/mix `get_path` | RFC-0043 / 0184 |
| surreal tx get/put/rmw/scan | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.11); get/scan `get_path` | RFC-0043 / 0184 |
| nebula neighbors/insert | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.12); neighbors `get_path` | RFC-0043 / 0184 |
| flink window / kafka changelog | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.13); window mix `get_path` | RFC-0043 / 0184 |
| ceph bluestore omap | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.14); read `get_path` | RFC-0043 / 0184 |
| solana shred/trailing | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.15); trailing `get_path` | RFC-0043 / 0184 |
| arango crud/traversal | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.16); traversal `get_path` | RFC-0043 / 0184 |
| venice fanout / rockstore | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.17); fanout `get_path` | RFC-0043 / 0184 |
| oxigraph spo/triple | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.18); lookup `get_path` | RFC-0043 / 0184 |
| rocksapi mixgraph/wbwi/compact/ingest | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.19); wbwi `get_path` | RFC-0043 / 0184 |
| ycsb_c_big 2^20 uniform get | **T**→tool | RFC-0059 | WRITEPHASE → `diagnose.lever` (0184 P2.20); `get_path` | RFC-0059 / 0184 |
| probe_hit p50 | **T**→tool | scale | `classify_get` p50 vs 0176 (0184 P2.21) | RFC-0184 |
| hydrate ingest | **T**→tool | scale | WRITEPHASE → `diagnose.lever` (0184 P2.22) | RFC-0184 |
| ycsb_b_mc4 | **U** | harness 95% get | `get_path`; no Linux 3-run cartaz; now in `BALANCE_SHAPES` (0184 P2.23) | RFC-0163 / 0184 |
| settle compact | **T**→tool | scale | WRITEPHASE → `diagnose.lever` (0184 P2.24) | RFC-0184 |
| pedra diagnose CLI JSON | **T**→tool | CLI | same `{"lever":…}` / `{"class":…}` as harness (0184 P2.25) | RFC-0184 |
| compare CLI diagnose JSON | **T**→tool | compare | `extract_cli_diagnose_lever` (0184 P2.26) | RFC-0184 |
| compare CLI get/probes class | **T**→tool | compare | `extract_cli_diagnose_class` (0184 P2.27) | RFC-0184 |
| compare CLI balance admits | **T**→tool | compare | `extract_cli_diagnose_admits` (0184 P2.28) | RFC-0184 |

## Próxima fase (rank da skill, 2026-09-07)

1. **U / Linux `overwrite_mc4` isolado + diagnose + `balance`** (caixa). Sem bake: não ratio-win Darwin. Skill **ainda implementa** o próximo furo local.
2. **Done this turn:** 0184 P2.28 — compare lê CLI `{"admits":…}`.
3. prefix 0.70× na caixa continua P1.2.

Não: skiplist. Não: n=50 merge. Não: turno vazio porque “é caixa”.
