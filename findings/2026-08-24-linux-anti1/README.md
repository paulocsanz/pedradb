# Bateria Linux/AMD — VM linux-anti-1 (2026-08-24)

Fonte `d22048e` (≈ `9edc373` + pernas RFC-0059 unif/big). Imagem
`ghcr.io/paulocsanz/pedradb-linux-gate:bench5anti`, build in-VM.

Host: AMD Ryzen Threadripper PRO 3975WX (4 vCPU, kernel 6.12.94-0-virt,
x86-64, região caixote `brasil`). Load gate armado (<2 duas vezes);
load1 1.65–1.77 durante toda a bateria (`watchdog.txt`).

Peer oficial: RocksDB default (`WriteOptions.sync=false`,
`peer_policy: rocks-default`). 3 rounds × (async+compat vs rocks;
kvr+compat vs rocks-kvr), ops=2000.

**Coluna medida (importante):** as pernas compat rodaram
`PEDRA_PARITY_ASYNC=1` — a coluna **async same-class** (WAL write, sem
fdatasync), a mesma coluna da bateria oficial do gate (rearm11 e
sucessoras). NÃO é a coluna G1 do produto ("fdatasync antes do Ok");
ratios aqui medem velocidade de motor, não o claim de durabilidade. O
campo `honesty` dentro dos `compare_report.json` arquivados ainda traz
o texto estático antigo ("fdatasync before Ok") — incorreto para esta
coluna; corrigido no bin em `4930f63` (honesty dinâmica).

## Resultado (gate `parity_gate_closed.py --from-compares`)

- **13/16 shapes oficiais PASS ≥2× (3/3 rounds)**: ycsb_a 2.65, ycsb_b
  2.51, ycsb_c 3.16, ycsb_d 2.93, ycsb_e 5.49, ycsb_f 2.14,
  deps_cache_overwrite 2.57, deps_lock_prewrite 2.02, deps_mvcc_latest
  2.95, kvrocks_get 4.70, kvrocks_set 2.27, kvrocks_scan 32.7,
  kvrocks_pipelined_set 3.51.
- **FAIL `deps_apply_batch`** 1.873 / 2.134 / 1.833 (2/3 rounds < 2.0;
  no Mac/rearm11 era 2.07–2.18).
- **FAIL `deps_raftlog`** mediana 0.910 (rounds 0.785/0.910/1.465; regra
  raftlog >1.0; no Mac era 1.07). Pedra perde por pouco no Linux/AMD.
- OPEN (não-gated): deps_scan 2.0–2.4, kvrocks_blob_set 0.85–1.4,
  kvrocks_set_mc50 2.7–3.2.

## P1.3 anti-overindex (Linux)

| perna | mediana ratio | contraparte zipf | leitura |
|---|---|---|---|
| ycsb_b_unif | 2.526 | ycsb_b 2.507 | +0.8% — sem queda |
| ycsb_c_unif | 3.112 | ycsb_c 3.160 | −1.5% — sem queda |
| ycsb_c_big (2^20 keys) | 2.461 | ycsb_c 3.160 | −22% — dentro do corte de 30% |

**Conclusão: não há overindex** — o ratio sobrevive sem o hot-set zipfiano
e com working set 1024× maior.

Round 2 mostrou ratios anômalos (ycsb_a 8.75×) com load limpo: o lado
RocksDB variou (compaction própria entre rounds), não contaminação
externa; `loads.txt`/`watchdog.txt` arquivados.

## Diagnóstico em curso

`deps_raftlog`/`deps_apply_batch` reprovam só no Linux/AMD (no Mac
passam). VM linux-diag-1 (imagem bench6diag) roda os 4 shapes isolados
com telemetria de fases (prepare/wal/mem/publish/flsh/lock_wait) +
teste H1 do tail do memtable (`ROCKS_DEPS_FOLD_TAIL`).
