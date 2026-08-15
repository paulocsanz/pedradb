# RFC-0026 P0.2 — Zipf vlog rewrite (this machine)

**Date:** 2026-08-14  
**Command:** `cargo bench -p pedradb-core --bench vlog_zipf -- --keys 2000 --val 4096 --passes 3 --seed 42`  
**Raw:** [`stdout.json`](stdout.json)

Not a HashKV reproduction (40 GiB, 6× SSD RAID). Shape only: 2000 keys × 4 KiB, Zipf s=0.99, `latest_only` then `compact_vlog`.

| Phase | update ms | gc ms | ratio before GC | rewrite |
|-------|----------:|------:|----------------:|---------|
| load | 8266 | — | 0.998 | — |
| Zipf 1 | 11785 | 86 | **0.499** | 16.4 → 8.2 MiB |
| Zipf 2 | 19004 | 168 | 0.499 | 16.4 → 8.2 MiB |
| Zipf 3 | 22336 | 116 | 0.499 | 16.4 → 8.2 MiB |

SST remap per GC ≈ 36 KiB (`bytes_written_sst` delta). GC ≪ update. See RFC-0026 P0.3.
