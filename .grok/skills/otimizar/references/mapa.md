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
| get 50M/100M clock | **S** | calibrated | class best/happy vs 0176 | RFC-0176 |
| hydrate 25M/100M | **S** | Linux >1× | — | RFC-0162 |
| **overwrite_mc4 25M** | **U** | Linux 0.557× 3/3; Darwin 0180 ~1.00 mediana (named 0.816); **caixa pós-0180 none** | unknown on Linux; Darwin avg_group 2.46 | RFC-0178 P1.3 / 0180 P1.1 / 0184 P1.1 |
| ycsb_f_mc4 3/3 intra | **W** | Linux mediana 1.47; run2 0.766 | not yet diagnose | RFC-0178 P1.4 |
| prefix 100M 4 GiB | **W** | caixa 0.70× | bounded-cache scan (not walk-all until cost says) | RFC-0178 P1.2 |
| probe_miss | **W** | 0.27× | miss path | RFC-0178 P1.1 / 0167 |
| 1c overwrite Darwin | **C**/DIAG | 0.845× | `wal_encode_or_write` despark=0 | RFC-0183 / 0184 |
| apply_mc4 Darwin async | **C**/DIAG | 0.478× (pré P0.5) | `flush_check`; 0184 P0.5 default-over parks whole mem O(1) | RFC-0183 / 0184 P0.5 |
| kvrocks_set_mc50 | **C** | 0.37× | `lock_convoy` Adaptive off n≥16 | RFC-0178 / 0183 |
| G1 1c write-per-op | **C** | fd-ceiling | one barrier/op | Agents.md / floor1x-g1 |
| ycsb_a/f_mc4 Darwin mixed | **T**→tool | 0.58–0.91 DIAG | `get_path` with `--read-pct 50`; `balance_admits=0` (DIAG); compare `diagnose.lever` (0184 P1.2) | RFC-0182 / 0184 |
| Linux async apply_mc4 | **U** | G1 2.79× exists; same-class mc4 not in floor1x 15 | — | RFC-0184 |
| 1B get @64 GiB | **S** (model) | happy ~61 µs; as-is 12.3 ms | `pedra diagnose get` | RFC-0176 |

## Próxima fase (rank da skill, 2026-09-07)

1. **U / Linux `overwrite_mc4` isolado + diagnose + `balance`** (caixa). Sem bake: não ratio-win Darwin. Skill **ainda implementa** o próximo furo local.
2. **Done this turn:** 0184 P1.2 — `benches[].diagnose.lever` no JSON; compare copia para a row. WRITEPHASE também no mc loop (A/F `read_pct`).
3. prefix 0.70× / probe_miss — reads, caixa.

Não: skiplist. Não: n=50 merge. Não: turno vazio porque “é caixa”.
