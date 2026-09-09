# ycsb_f_mc4 Darwin DIAG (fire 70)

**When:** 2026-09-08  
**Peer:** same-class async Pedra vs Rocks `sync=false`.

100k zipfian, 50k ops × 4 clients, `MC_FRESH=1`.

`ratio=0.969` pedra=261151 rocks=269590. Pedra p50 8.6 µs vs Rocks 4.8 µs;
Pedra p99 60 µs vs Rocks 94 µs.

Linux mediana 1.47 / run2 **0.766×** still unpaid. No engine cut this fire
(100k Darwin ≈1× is not the Linux named loss).
