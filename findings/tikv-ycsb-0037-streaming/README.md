# RFC-0037 P0.2 remesure — streaming L0 compact vs Rocks fdatasync

4096/2000 zipfian 1 KB, `ycsb,deps` combinada, `ROCKS_PARITY_FULL_SYNC=0`.
Três runs seguidas. Pedra apply é estável; o Rocks apply não.

| run | Pedra apply | max | Rocks apply | max | ratio | ≤2× |
|---|---:|---:|---:|---:|---:|:---:|
| p03 (fria) | 1 949 | 157 ms | 2 791 | 136 ms | 0.70 | sim |
| p03b | 1 937 | 93 ms | 3 623 | 32 ms | 0.54 | sim |
| **p03c (oficial)** | **1 835** | **105 ms** | **5 576** | **0.55 ms** | **0.33** | **não** |

p03c é o peer limpo (Rocks apply p50 174 µs, p95 209 µs, max 0.55 ms — sem compact no caminho).
Não usar p03/p03b como 11/11: o Rocks estava lento.

Tabela oficial e JSON: `compat.json` / `rocks-fd.json` / `compare.json` (= p03c).
Raw das três: `p03*.json`.

Streaming cortou a cauda Pedra (max 221 ms em 0036 → 93–157 ms) e **não** o qps
(~1 900, igual a `6a4267b`). O que resta no put é recodificar+lz4+`fdatasync` do L1.
P0.3 11/11 **não** fechou. Próximo honesto: P2.1 worker fora do core.
