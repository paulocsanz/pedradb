# RFC-0042 P1 — bound do catch-up + sweep da janela + corte não-fd

2026-08-18 06:09–06:18. Sweep de `PEDRA_CATCHUP_US` (0/10/15/25/50) com o
bound P1.1 ativo (`min(window, fd_ema/2)`) e o encode direto P1.3, sob a
forma oficial (SYNC=0, CLIENTS=4, intercalado por run, mediana de 3).
Binários congelados em `implementer/bin-p11` (features `real`). Peers
`sync:false` em todas as 15 runs (`sweep-us*/run*/compare/`).

Mesma ressalva de ambiente do P0: load average ~42 (fuzzing + Spotlight)
durante todo o sweep — ratios absolutos não são comparáveis com head3;
comparações são válidas entre configs do sweep (mesma hora, mesma carga,
intercaladas).

## Agrupamento (contadores — imunes a ruído de parede)

| config | avg_group (3 runs) |
|---|---|
| us=0 | 1.19 / 1.21 / 1.21 |
| us=10 | 1.45 / 1.45 / 1.43 |
| us=15 | 1.44 / 1.43 / 1.49 |
| us=25 | 1.44 / 1.48 / 1.43 |
| us=50 | 1.42 / 1.41 / 1.50 |

Com o bound, toda janela ≥ fd/2 (~12 µs nesta caixa) converge: a escolha
entre 10/15/25/50 é irrelevante. Só há duas políticas reais: off (absorção
tardia apenas) vs on.

## Medianas (ratios; compat qps mediano nas 3 gates)

| config | ycsb_a_mc4 | ycsb_f_mc4 | deps_cache_overwrite_mc4 | qps (a/f/ow) |
|---|---:|---:|---:|---|
| us=0 | **0.432** | **0.341** | **0.320** | 16.1k / 9.2k / 7.9k |
| us=10 | 0.233 | 0.183 | 0.098 | 5.5k / 4.4k / 3.7k |
| us=15 | 0.146 | 0.120 | 0.203 | 6.9k / 8.4k / 7.1k |
| us=25 | 0.203 | 0.219 | 0.221 | 7.2k / 6.3k / 9.8k |
| us=50 | 0.358 | 0.166 | 0.184 | 10.8k / 8.1k / 3.8k |

Scatter por run é alto (caixa saturada), mas `us=0` ganha a mediana nas
três shapes gate, em ratio e em qps do compat.

## Decisão do default (P1.2)

**`CATCHUP_WINDOW_DEFAULT` 50 µs → 0** (off; absorção tardia continua).

1. Sweep de hoje: `us=0` melhor mediana nas 3 gates (ratio e qps).
2. RFC-0041 P1.1 probe (caixa silenciosa) já media catch-up 0 = 217 µs vs
   catch-up 50 = 244 µs por apply-op — a janela nunca se pagou nem quiet.
3. Com o bound, janela > 0 adiciona apenas latência de espera em caixas
   saturadas (chegadas atrasam além de fd/2 e a espera vira perda seca).
4. O floor 2.0 do RFC-0041 não depende da janela: `deps_apply_batch_mc4`
   (64 ops ≥ `CATCHUP_SKIP_OPS`=32) pula a espera em qualquer config.
5. `PEDRA_CATCHUP_US` continua honrado para opt-in (com bound fd/2).

## P1.3 — corte não-fd (encode direto no frame)

Corte: `Wal::encode_write_op_batches` não passa mais pelo scratch
`logical` (`encode_ops` → memcpy → `fragment_record` → memcpy); campos
vão direto ao frame via `EncodedOpsSource` (uma cópia). Byte-identidade
assegurada por `fragment_encoded_matches_scratch_path_bytes` (multi-bloco,
offsets no meio do bloco, campos vazios) + suítes `wal::` (33) e
`concurrent::` (41) verdes.

Antes/depois (`lone_split`, mesma caixa — **sob load ~42, ruído domina**):
baseline `../rfc0042-p0/lone-split-baseline.txt` io 58.6–88.3 µs/op;
`lone-split-after-p13.txt` io 98.7–99.5 µs/op (fd da caixa piorou entre
as medições; microbench cru p99 207→950 µs). A economia esperada
(~1 memcpy de 1 KiB ≈ 0.03–0.1 µs/op) está abaixo do ruído medido — o
efeito de performance é registrado como **neutro**; o benefício é menos
uma alocação/cópia por membro de grupo, e o 1c permanece ~98% fd
(teto `1/t_fd` registrado no RFC).

## Estado do gate 0.8

Sob a carga de hoje, **nenhuma** config atinge 0.8 nas `_mc4` (melhor:
us=0 → 0.432/0.341/0.320). A P2.1 remensura com default novo (0) e
registra o que fechar; a limitação da caixa entra no README do P2.
