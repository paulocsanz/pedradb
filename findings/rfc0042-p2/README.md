# RFC-0042 P2 — remesura oficial final (mecanismos P1.1+P1.2+P1.3)

2026-08-18 06:31–06:33. Forma oficial: 16 shapes × 3 runs intercalados,
`ROCKS_PARITY_SYNC=0`, `ROCKS_PARITY_CLIENTS=4`, records=4096 / ops=2000 /
payload=1000 / zipfian, peer RocksDB default async-WAL (`sync:false` nas 3
runs). Binários congelados em `implementer/bin-p2` = código final do RFC
(bound fd/2 ativo, `CATCHUP_WINDOW_DEFAULT=0`, encode direto P1.3 com o fix
de split de campos fixos), sem `PEDRA_CATCHUP_US` no ambiente — o default
medido é o default entregue. (Uma primeira rodada final com binários
pré-fix foi descartada e re-rodada; o fix muda layout de bytes em disco,
não o custo do caminho de escrita.)

## Ambiente

Load average ~42 durante as 3 runs (fuzzing do usuário + Spotlight; 12
CPUs lógicas). Scatter alto em ambos os lados (ex.: `ycsb_b`
1.33→0.11 entre runs). Comparações válidas: baseline/sweep/final deste
mesmo dia sob a mesma carga. **Não comparar com head3 (caixa silenciosa).**

## Medianas (`medians.txt`; formato head3)

| gate (0.8) | mediana | compat qps | rocks qps |
|---|---:|---:|---:|
| `ycsb_a_mc4` | 0.232 | 11,959 | 52,914 |
| `ycsb_f_mc4` | 0.288 | 11,819 | 27,051 |
| `deps_cache_overwrite_mc4` | 0.350 | 9,509 | 27,138 |

**O gate 0.8 não foi atingido nesta remesura.** Sem mover o gate nem o
protocolo: o número vai para a status table como medido.

Leitura: o lado compat é estável entre remesuras do mesmo dia (a_mc4
12.4k→12.0k; f_mc4 11.5k→11.8k; ow_mc4 7.8k→9.5k) — quem oscila é o
Rocks (a_mc4 17.2k→52.9k), com a caixa a ~3.5× do número de CPUs. Com 4
clientes e absorção apenas (avg_group 1.15–1.20), cada op espera fd's dos
outros ~serialmente (4 × ~30 µs ≈ 120 µs/op ⇒ ~10–12k qps — exatamente o
medido). Janela ligada não resgata nesta caixa: o sweep (`../rfc0042-p1/`)
mediu us≥10 pior que us=0 nas três gates (chegadas atrasam além de fd/2 e
a espera vira perda seca). Em caixa silenciosa o bound deve coalescer de
verdade; revalidação em caixa silenciosa é o follow-up registrado no RFC.

Destaques fora do gate: `deps_apply_batch_mc4` **2.630** (floor 2.0 do
RFC-0041 mantido); `deps_raftlog_mc4` **1.303**; `deps_scan` **2.771**;
`ycsb_c` **2.084**; 8/16 shapes ≥ 0.8.

## Antes/depois (mesmo dia, mesma carga)

| shape | baseline P0 (janela 50 fixa, sem bound) | final P2 | nota |
|---|---:|---:|---|
| ycsb_a_mc4 | 0.118 | 0.232 | compat estável; Rocks variou |
| ycsb_f_mc4 | 0.308 | 0.288 | — |
| deps_cache_overwrite_mc4 | 0.143 | 0.350 | compat 7.8→9.5k |
| deps_raftlog_mc4 | 0.702 | 1.303 | maior ganho estável |
| deps_apply_batch_mc4 | 2.080 | 2.630 | floor mantido |
| ycsb_b | 0.190 | 0.236 | scatter ±0.5 na caixa |
| ycsb_d | 0.410 | 0.440 | idem |
| ycsb_a (1c) | 0.156 | 0.434 | idem |
| ycsb_f (1c) | 0.186 | 0.320 | idem |
| deps_cache_overwrite (1c) | 0.136 | 0.294 | idem |

Com scatter da caixa de ±0.3–0.5 em shapes de escrita, só os ganhos
maiores que isso são atribuíveis ao mecanismo (`deps_raftlog_mc4`,
`deps_cache_overwrite_mc4`, `deps_apply_batch_mc4`).

## 1c — teto físico confirmado

Split do lone-writer (P0.2 + P1.3): non-fd ≈ 1.0–2.1 µs/op, io (fd)
≈ 98%. 0.8 nas 1c precisaria de 76–162k qps; o teto `1/t_fd` desta caixa
sob load é ~10–17k (lone_split pós-P1.3: ceiling 9.98–9.48k; fd p50
32–37 µs, p99 até 950 µs). 0.8 em 1c segue fora de alcance sob G1 —
como registrado no RFC desde o P0.
