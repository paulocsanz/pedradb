# rearm10 — bateria quieta do branch `rfc-0054-gaps` (P1.1 mvcc reverso + P1.3 scan step_user)

**OFICIAL** para RFC-0054 P1.1. Gate load1<10 ×2 (armou 16:36:58); pernas
em load 6,79–8,41; peers `sync:false`, `peer_policy:rocks-default`,
`ROCKS_PARITY_SYNC=0`, `ROCKS_YCSB_OPS=2000` — método rearm7/8/9.
(Tentativa anterior abortada: rebuild plain sobrescreveu o binário
`--features real` e a perna rocks morreu com "needs --features real";
rebuild com features e binário dedicado `rocks-parity-bench-real-p11`.)

- HEAD medido: `rfc-0054-gaps` @ d36f747 (mvcc reverse-walk 974d8ff +
  count step_user d36f747)
- 3 rodadas × 4 pernas, JSONs em `r{1,2,3}/`

## Resultado — RFC-0054 P1.1 `deps_mvcc_latest` **FECHA 3/3 ≥2×**

| rodada | pedra qps | rocks qps | ratio |
|---|---:|---:|---:|
| r1 | 760.721 | 348.951 | **2,180** |
| r2 | 865.333 | 342.361 | **2,528** |
| r3 | 982.439 | 329.487 | **2,982** |

rearm9 (pré-fix): 1,41. O caminho (`last_under_user_prefix` →
`last_visible_under_prefix`) coletava TODAS as versões do usuário num Vec
por chamada; agora é max-of-maxes reverso por conjunto (map + shards do
tail_idx) sem materializar.

`deps_raftlog` re-confirma >1× 3/3: 1,024/1,354/0,999 (r2 rocks 96 k é
ruído do peer; pedra 130–133 k estável).

## Tabela de medianas (3 rodadas, ycsb+deps)

| forma | pedra | rocks | ratio | rearm9 |
|---|---:|---:|---:|---:|
| ycsb_e | 1.189.237 | 236.061 | 5,04 | 5,14 |
| deps_cache_overwrite | 1.824.956 | 381.934 | 4,78 | 4,39 |
| ycsb_d | 8.443.272 | 1.769.129 | 4,77 | 4,67 |
| ycsb_a | 3.017.920 | 655.702 | 4,60 | 4,44 |
| ycsb_c | 9.422.850 | 2.256.807 | 4,18 | 4,08 |
| ycsb_b | 6.232.160 | 1.741.907 | 3,58 | 3,50 |
| ycsb_f | 1.702.187 | 537.297 | 3,17 | 3,08 |
| **deps_mvcc_latest** | **865.333** | **342.361** | **2,53 (3/3)** | 1,41 |
| deps_lock_prewrite | 39.776 | 19.164 | 2,08 (2/3) | 1,96 |
| deps_apply_batch | 21.157 | 11.568 | 1,83 | 1,93 |
| deps_scan | 472.274 | 269.191 | 1,89 (1,65/1,96/1,89) | 1,80 |
| deps_raftlog | 130.219 | 127.909 | 1,02 3/3 ≥1 | 1,05 |

## kvrocks (pernas kvr)

| forma | ratios | estado |
|---|---|---|
| kvrocks_get | 6,22/6,14/5,85 | estável |
| kvrocks_set | 20,13(!)/4,05/4,67 | r1 anômalo do peer |
| kvrocks_set_mc50 | 1,88/1,97/1,79 | borda |
| kvrocks_blob_set | 1,23/1,27/1,31 | P1.2 aberto (banda simétrica) |

## Abertos após rearm10

- **P1.3** deps_scan 1,89 (step_user +10% não bastou; TLS absolve ~45%
  das scans, 874/2000 vão ao kernel)
- **P1.4** apply 1,83; lock_prewrite 2,08 mas 2/3
- **P1.2** blob 1,23–1,31 (16 KiB SET = banda; peer acelerou no rearm9)
