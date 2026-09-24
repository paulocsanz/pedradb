# Mapa — forte / fraco / teto / buraco de ferramenta

**Updated:** 2026-09-08
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
| **overwrite_mc4 25M** | **U** | Linux 0.557× 3/3; P0.78 `write_buffer_for_ram` 64 MiB below 8 GiB (4 GiB caixa matches Rocks default memtable). Leftover+L0 2M 4GiB-shaped DIAG Pedra **227 k / p50 16.1 µs** (P0.75 was 209 k / 17.4). Darwin 100k DIAG **0.716×** (164 k vs 229 k, p50 21.3 vs 13.2). Caixa 3-run after P0.78 unpaid | 4 GiB mem+leftover; Darwin p50 21 vs 13 | RFC-0178 P1.3 / 0180 P0.62–P0.78 |
| rockset_hybrid | **U** | Darwin DIAG **0.811×** (115 k vs 142 k; was 0.123 serial-put); no Linux 3-run | 1c batch+get. Static `async_wal` (`--clients 1 --read-pct 11`). WRITEPHASE on batch still T. | RFC-0043 / 0184 P2.37 |
| rockset_hybrid_mc4 | **U** | Darwin DIAG **0.543×** (29.0 k vs 53.3 k); no Linux 3-run | ingest WriteBatch + point get, 4 clients. 1c was 0.811. p50 60 vs 38 µs. | RFC-0184 P2.43 |
| ycsb_f_mc4 3/3 intra | **W** | Linux mediana 1.47; run2 0.766; Darwin DIAG **0.638×** (214 k vs 336 k, p50 7.8 vs 4.0 µs, 100k zipf). Prior 0.969 was vs Rocks 270 k | `get_path` + rmw get+put; Pedra max 72 vs 18 ms | RFC-0178 P1.4 / P0.13 |
| prefix 100M 4 GiB | **W** | caixa 0.70×; P0.78/0173 P2.4 keep ram/4 newest SST pages (was DONTNEED-all). Gerador 100M@4GiB `dominant=pread` legal 14 µs vs as_is 1.24 ms | bounded-cache scan; keep newest pages | RFC-0178 P1.2 / 0173 P2.4 / 0184 |
| probe_miss | **W** | 0.27× | miss path; packed envelope 0167; mem-live envelope (0178 P0.14); overlapping by_lo_rank (P0.15). Caixa 3-run still P1.1 | RFC-0178 P0.15 / P1.1 / 0167 / 0184 |
| 1c overwrite Darwin | **C**/DIAG | 0.845× | `wal_encode_or_write` despark=0 | RFC-0183 / 0184 |
| apply_mc4 Darwin async | **C**/DIAG | 0.478× (pré P0.5) | `flush_check`; 0184 P0.5 default-over parks whole mem O(1) | RFC-0183 / 0184 P0.5 |
| kvrocks_set_mc50 | **C** | 0.37× | `lock_convoy` Adaptive off n≥16; WRITEPHASE → `diagnose.lever` (0184 P2.3) | RFC-0178 / 0183 / 0184 |
| G1 1c write-per-op | **C** | fd-ceiling | one barrier/op | Agents.md / floor1x-g1 |
| ycsb_a/f_mc4 Darwin mixed | **T**→tool | 0.58–0.91 DIAG | `get_path` with `--read-pct 50`; `balance_admits=0` (DIAG); compare `diagnose.lever` (0184 P1.2) | RFC-0182 / 0184 |
| Linux async apply_mc4 | **U** | G1 2.79× exists; same-class mc4 not in floor1x 15 | — | RFC-0184 |
| 1B get @64 GiB | **S** (model) | happy ~61 µs; as-is 12.3 ms | `pedra diagnose get` | RFC-0176 |
| lookup_100 / get_loop | **T**→tool | Darwin 427 µs @100M | scale `classify_get` ns/100 vs 0176 (0184 P2.7) | RFC-0178 P0.9 / 0184 |
| qs_hot_get / qs_neg / qs_batch_write | **T**→tool | RFC-0043 HL | WRITEPHASE → `diagnose.lever` (0184 P2.8); hot/neg `get_path`. mc4 cartaz: hot 0.719 / neg 0.808 / batch 1.021 DIAG | RFC-0043 / 0184 |
| kvrocks 1c get/set/scan | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.9); get/scan `get_path`. mc4 GET 1.205 / SCAN 1.687 / pipelined-set 0.807 / blob-set 0.766 DIAG | RFC-0043 / 0184 |
| kvrocks_get_mc4 | **U** | Darwin DIAG **1.205×** (5.32 M vs 4.42 M); no Linux 3-run | redis GET, 4 clients. Rocks 4.42 M not collapsed. Not Linux cartaz. | RFC-0184 P2.46 |
| myrocks + linkbench_mix | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.10); select/range/mix `get_path`. mc4 point-select 0.430; read_only 1.870 (host-default JSON, not a win) | RFC-0043 / 0184 |
| myrocks_point_select_mc4 | **U** | Darwin DIAG **0.430×** (2.12 M vs 4.94 M); no Linux 3-run | oltp_point_select, 4 clients. p50 0.5 vs 0.7 µs; hole is tail. JSON host-default tag; timed is GET. | RFC-0184 P2.47 |
| surreal tx get/put/rmw/scan | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.11); get/scan `get_path`. mc4 GET DIAG 1.364 (host-default JSON, not a win) | RFC-0043 / 0184 |
| nebula neighbors/insert | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.12); neighbors `get_path`. mc4 DIAG neighbors 0.776 / insert 1.091 | RFC-0043 / 0184 |
| nebula_get_neighbors_mc4 | **U** | Darwin DIAG **0.776×** (1.56 M vs 2.01 M); no Linux 3-run | GO 1-hop prefix scan, 4 clients. p50 0.5 vs 1.8 µs; hole is tail. Same-class async. | RFC-0184 P2.48 |
| arango_traversal_mc4 | **U** | Darwin DIAG **0.003×** (3.44 k vs 1.05 M); no Linux 3-run | 2-hop prefix scan, 4 clients. p50 2.3 vs 3.5 µs; hole is tail (wall 58 vs 0.19 s). Rocks 1.05 M not collapsed. Repeat 0.003. | RFC-0184 P2.49 |
| surreal_tx_get_mc4 | **U** | Darwin DIAG **1.364×** (364 k vs 267 k); JSON host-default; no Linux 3-run | snapshot get+commit, 4 clients. Not a published win vs Rocks default. p50 10 vs 12 µs. | RFC-0184 P2.50 |
| oxigraph_spo_lookup_mc4 | **U** | Darwin DIAG **0.985×** (4.28 M vs 4.34 M); no Linux 3-run | SPO point get, 4 clients. Same-class async. p50 0.6 vs 0.8 µs. | RFC-0184 P2.51 |
| solana_trailing_read_mc4 | **U** | Darwin DIAG **2.414×** (2.34 M vs 968 k); no Linux 3-run | 25-key trailing scan, 4 clients. Same-class async. Not Linux cartaz. | RFC-0184 P2.52 |
| kvrocks_scan_mc4 | **U** | Darwin DIAG **1.687×** (1.75 M vs 1.04 M); no Linux 3-run | SCAN COUNT=25, 4 clients. Same-class async. Not Linux cartaz. | RFC-0184 P2.53 |
| flink_window_state_mc4 | **U** | Darwin DIAG **0.522×** (110 k vs 211 k); no Linux 3-run | put + 25-key window scan, 4 clients. Same-class async. | RFC-0184 P2.54 |
| kafka_changelog_flush_mc4 | **U** | Darwin DIAG **0.897×** (50.8 k vs 56.7 k); no Linux 3-run | concurrent WriteBatch ingest (no per-op flush). Same-class async. | RFC-0184 P2.55 |
| bluestore_omap_read_mc4 | **U** | Darwin DIAG **0.828×** (1.12 M vs 1.35 M); JSON host-default; no Linux 3-run | get+8-key scan, 4 clients. Not a published win vs Rocks default. | RFC-0184 P2.56 |
| myrocks_read_only_mc4 | **U** | Darwin DIAG **1.870×** (1.94 M vs 1.04 M); JSON host-default; no Linux 3-run | 25-key PK range, 4 clients. Not a published win vs Rocks default. | RFC-0184 P2.57 |
| wbwi_read_your_writes_mc4 | **U** | Darwin DIAG **0.410×** (6.54 M vs 16.0 M); no Linux 3-run | WBWI overlay-get, 4 clients. Pedra real WBWI; Rocks adapter is last-write-wins overlay (no rust-rocksdb WBWI). p50 0.4 vs 0.2 µs. Same-class async. | RFC-0184 P2.58 |
| mixgraph_like_mc4 | **U** | Darwin DIAG **1.154×** (108 k vs 93.8 k); no Linux 3-run | put+2get+seek, 4 clients. p50 33 vs 22 µs (Pedra loses p50). Same-class async. Not Linux cartaz. | RFC-0184 P2.59 |
| oxigraph_triple_put_mc4 | **U** | Darwin DIAG **0.898×** (32.2 k vs 35.9 k); no Linux 3-run | triple WriteBatch×32, 4 clients. p50 123 vs 98 µs. Rocks 1.15 M puts/s not collapsed. Same-class async. | RFC-0184 P2.60 |
| nebula_insert_edge_mc4 | **U** | Darwin DIAG **1.091×** (33.2 k vs 30.4 k); no Linux 3-run | edge WriteBatch×32, 4 clients. p50 121 vs 100 µs (Pedra loses p50). Same-class async. Not Linux cartaz. | RFC-0184 P2.61 |
| solana_shred_append_mc4 | **U** | Darwin DIAG **1.296×** (84.9 k vs 65.5 k); no Linux 3-run | WriteBatch×16 unique shreds, 4 clients. p50 44 vs 36 µs (Pedra loses p50). Same-class async. Not Linux cartaz. | RFC-0184 P2.62 |
| arango_doc_crud_mc4 | **U** | Darwin DIAG **0.445×** (239 k vs 537 k); no Linux 3-run | 50% get / 30% put / 20% 5-key scan, 4 clients. p50 5.1 vs 2.1 µs. Rocks 537 k not collapsed. Same-class async. | RFC-0184 P2.63 |
| kvrocks_pipelined_set_mc4 | **U** | Darwin DIAG **0.807×** (29.9 k vs 37.0 k); no Linux 3-run | WriteBatch×32 zipf SETs, 4 clients. p50 118 vs 79 µs. Rocks 1.18 M puts/s not collapsed. Same-class async. | RFC-0184 P2.64 |
| kvrocks_blob_set_mc4 | **U** | Darwin DIAG **0.766×** (46.3 k vs 60.5 k); no Linux 3-run | 16 KiB BlobDB-sized SET, 4 clients. Rocks 60 k 16 KiB SETs not collapsed. Same-class async. | RFC-0184 P2.66 |
| deps_lock_prewrite_mc4 | **U** | Darwin DIAG **1.006×** (20.1 k vs 20.0 k); no Linux 3-run | lock+default WriteBatch×32, 4 clients. p50 121 vs 169 µs (Pedra wins p50). Pedra max 3.6 s vs 55 ms — not a quiet win. Same-class async. | RFC-0184 P2.67 |
| flink window / kafka changelog | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.13); window mix `get_path`. flink mc4 0.522; kafka mc4 0.897 | RFC-0043 / 0184 |
| ceph bluestore omap | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.14); read `get_path`. mc4 DIAG 0.828 (host-default JSON, not a win) | RFC-0043 / 0184 |
| solana shred/trailing | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.15); trailing `get_path`. mc4 cartaz: trailing 2.414 / shred-append 1.296 DIAG | RFC-0043 / 0184 |
| arango crud/traversal | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.16); traversal `get_path`. mc4 cartaz: traversal 0.003 / crud 0.445 DIAG | RFC-0043 / 0184 |
| venice fanout / rockstore | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.17); fanout `get_path`. mc4 cartaz: fanout 0.735 / rockstore 0.566 DIAG | RFC-0043 / 0184 |
| venice_fanout_get_mc4 | **U** | Darwin DIAG **0.735×** (102 k vs 139 k); no Linux 3-run | 32 point-gets/op, 4 clients. p50 32 vs 27 µs; hole is tail. | RFC-0184 P2.45 |
| rockstore_widecol_rw_mc4 | **U** | Darwin DIAG **0.566×** (200 k vs 353 k); no Linux 3-run | 50% put / 50% col-prefix scan, 4 clients. p50 15.2 vs 5.5 µs. Rocks 353 k mix-ops not collapsed. Same-class async. | RFC-0184 P2.65 |
| oxigraph spo/triple | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.18); lookup `get_path`. mc4 cartaz: spo 0.985 / triple-put 0.898 DIAG | RFC-0043 / 0184 |
| rocksapi mixgraph/wbwi/compact/ingest | **T**→tool | RFC-0043 | WRITEPHASE → `diagnose.lever` (0184 P2.19); wbwi `get_path`. mc4 cartaz: wbwi 0.410 / mixgraph 1.154 DIAG | RFC-0043 / 0184 |
| ycsb_c_big 2^20 uniform get | **T**→tool | RFC-0059 | WRITEPHASE → `diagnose.lever` (0184 P2.20); `get_path` | RFC-0059 / 0184 |
| probe_hit p50 | **T**→tool | scale | `classify_get` p50 vs 0176 (0184 P2.21) | RFC-0184 |
| hydrate ingest | **T**→tool | scale | WRITEPHASE → `diagnose.lever` (0184 P2.22) | RFC-0184 |
| ycsb_b_mc4 | **U** | Darwin DIAG **0.060×** (253 k vs 4.2 M, tiny); no Linux 3-run | 95% get. Static `get_path`. p50 can beat the 0176 clock; QPS loss is tail. | RFC-0163 / 0184 P2.36 / P2.37 |
| ycsb_c_mc4 | **U** | Darwin DIAG **0.909×** (4.71 M vs 5.18 M, 100k zipf); no Linux 3-run | 100% get mc4. p50 tied 0.5 µs; 9% hole is tail (max 239 vs 75 µs). Do not overfit this size. | RFC-0184 P2.39 |
| qs_hot_get_mc4 | **U** | Darwin DIAG **0.719×** (2.09 M vs 2.90 M); no Linux 3-run | 99% get hot 10% + 1% batch. p50 0.7 vs 0.8 µs; hole is tail (p99 33 vs 24 µs). | RFC-0184 P2.40 |
| qs_neg_lookup_mc4 | **U** | Darwin DIAG **0.808×** (4.34 M vs 5.37 M); no Linux 3-run | 100% miss get mc4. p50 0.9 vs 0.6 µs. Same family as probe_miss W. | RFC-0184 P2.41 |
| qs_batch_write_mc4 | **U** | Darwin DIAG **1.021×** (32.7 k vs 32.0 k, load 12–15); no Linux 3-run | every op WriteBatch×32, 4 clients. Tied, not a quiet win. Pedra max 147 vs Rocks 17 ms. | RFC-0184 P2.42 |
| yugabyte_docdb_rmw | **U** | Darwin DIAG **0.732×** (404 k vs 552 k) | 70% overlay-get. Static `get_path`. | RFC-0043 / 0184 P2.37 |
| yugabyte_docdb_rmw_mc4 | **U** | Darwin DIAG **0.952×** (372 k vs 390 k); no Linux 3-run | 70% overlay-get / 30% RMW, 4 clients. 1c was 0.732. p50 tied ~2.4 µs. | RFC-0184 P2.44 |
| settle compact | **T**→tool | scale | WRITEPHASE → `diagnose.lever` (0184 P2.24) | RFC-0184 |
| pedra diagnose CLI JSON | **T**→tool | CLI | same `{"lever":…}` / `{"class":…}` as harness (0184 P2.25) | RFC-0184 |
| compare CLI diagnose JSON | **T**→tool | compare | `extract_cli_diagnose_lever` (0184 P2.26) | RFC-0184 |
| compare CLI get/probes class | **T**→tool | compare | `extract_cli_diagnose_class` (0184 P2.27) | RFC-0184 |
| compare CLI balance admits | **T**→tool | compare | `extract_cli_diagnose_admits` (0184 P2.28) | RFC-0184 |
| pedra diagnose get JSON clock | **T**→tool | CLI | 0176 best/happy/worst/as_is in JSON (0184 P2.29) | RFC-0184 |
| pedra diagnose probes JSON | **T**→tool | CLI | per_get/p_best in JSON (0184 P2.30) | RFC-0184 |
| diagnose JSON gap/timed | **T**→tool | kernel | json_object gap_ns+timed_ns (0184 P2.31) | RFC-0184 |
| pedra diagnose balance shapes | **T**→tool | CLI | JSON shapes=BALANCE_SHAPES (0184 P2.32) | RFC-0184 |
| snapshot get_hit 1M | **T**→tool | snapshot-bench | `classify_get` vs 0176 (0182 P2.1) | RFC-0182 |
| snapshot lookup_100 | **T** | snapshot-bench | no `classify_get` yet (loop/100) | RFC-0182 |
| snapshot prefix_scan | **T** | snapshot-bench | no `classify_probes` yet | RFC-0182 |
| snapshot probe_miss / probe_hit | **T** | snapshot-bench | no `classify_get` / `classify_probes` yet — **não** clonar; scale já classifica | RFC-0182 |
| 1B get @64 GiB as-is | **C** (model) | gerador | `predict_get_bottleneck` = as_is_walk sem runtime (0184 P2.33); P_best=5 vs n_files=913 | RFC-0176 / 0184 |
| get composed spec | **T**→tool | gerador | `predict_get_composed` happy/capacity/cold; bloom reject ≠ pread; 50M legal < 0176 envelope (0184 P2.35) | RFC-0184 P2.35 |

## Próxima fase (rank da skill, 2026-09-07)

1. **Done this turn:** 0178 P0.13 TLS precise n≤32 (`get_path` group). Caixa overwrite bake.
2. **W leftover:** `ycsb_f` still W until Linux 3-run (P1.4); `probe_miss` 0.27× publicable until caixa (HEAD envelope 2.71× DIAG). overwrite_mc4 caixa.
3. Não: skiplist; T-clone; Darwin win.

Não: skiplist. Não: n=50 merge. Não: turno vazio porque “é caixa”.
