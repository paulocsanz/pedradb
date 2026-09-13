# RFC-0211 P2.1 — Sweep do eixo writers (mc2/3/6/8): fronteira do drenar-grupo

Data: 2026-09-13T03:33:27Z · binário `052d151e` (sha-c6244132) · 3 rodadas ·
`PEDRA_PARITY_ASYNC=1` · peer rocks default `sync=false` · stats ON

**Regime e rótulo:** DIAG Darwin (disco real, admissão probe-Ok — sem
escada; `flsh≈0` no APFS). O meter oficial stats-off no gate Linux está
**gate-blocked** (registro datado: `{SCRATCH}/gate-blocked.txt`; deploy
`:p211v` amd64 pending no p149 desde 03:07Z, único host da região brasil
desconectado). Dados crus: `{SCRATCH}/p211q-host-run.log`.

## Resultado: a fronteira do escalonador rmw cruza entre mc3 e mc6

FRONTIER ycsb_f (rmw/min ÷ clean/min), min-of-3:

| writers | clean | rmw | delta |
|---|---|---|---|
| mc2 | 0,392 | 0,353 | 0,90× |
| mc3 | 0,255 | 0,254 | 0,99× |
| mc6 | 0,368 | **0,726** | **1,97×** |
| mc8 | 0,329 | **0,733** | **2,23×** |

Padrão idêntico em `ycsb_a` (mc6: 0,372→0,712; mc8: 0,339→0,838) e
`deps_cache_overwrite` (mc6: 0,378→0,830; mc8: 0,409→0,832). O anchor mc4
(0,491→0,836 = 1,70×) senta exatamente na subida da curva.

GUARD deps_apply_batch (multi-op, não pode regredir): deltas 0,91–1,13×
em todo o eixo — **guard passa** (sem regressão fora do ruído Darwin).

## Mecanismo (telemetria PHASE explica a fronteira)

- **Braço clean (default): o grupo NUNCA se forma.** `avg_grp=1.00` e
  `queued=0` em TODAS as contagens de writers (2→8), e o `lock_wait`
  CRESCE com concorrência: 2,1µs (mc2) → 6,5µs (mc3) → 19–21µs (mc6) →
  22–32µs (mc8). Writers serializam no lock do grupo sem cooperar.
- **Braço rmw: o grupo se forma quando há concorrência para agrupar.**
  `avg_grp`: 1,04 (mc2) → 1,12–1,14 (mc3) → 2,0–2,2 (mc6) → 2,9–3,1
  (mc8); `lwait` desaba para 0,2–1,8µs em todas as contagens. O `wal`
  por grupo (~3,5–4µs) é amortizado entre os membros (n= conta grupos
  líder: mc8 rmw 2.780–2.570 grupos × avg_grp≈3 ≈ 8.000 ops).
- A fronteira mc3→mc6 é exatamente onde `avg_grp` sai de ~1,1 para >2:
  com <4 writers quase não há sobreposição para drenar; com ≥6 há, e o
  escalonador converte lock_wait serializado em grupo amortizado.

## Consequência para o próximo RFC (nº 1 do inventário rev. 3)

O drenar-grupo é o mecanismo CERTeiro e agora com curva: paga a partir de
~4–6 writers (1,7–2,2×) e não regrediu multi-op. Mas o teto dele é o
**trabalho serial por grupo** — no Linux âncora, o fdatasync per-commit
da coluna de paridade (adjudicado na P1.2) limita o ratio absoluto
(rmw mc8 DIAG Darwin 0,73–0,84 com flsh≈0; na âncora mc4 0,836 com
fdatasync real). O próximo ataque precisa reduzir o custo serial por
grupo (fdatasync amortizado/agrupado no commit do grupo — mesmo batch,
uma barreira) e não mais mexer em escalonamento.

## Pendências honestas

- Meter oficial (gate Linux, stats-off, quiet): blocked por ambiente,
  registro datado acima; o deploy pending agenda sozinho quando o host
  brasil reconectar (monitor no ar) — os números DIAG acima serão
  confirmados/rejustrados na classe âncora quando isso ocorrer.
- Ratios Darwin sob host compartido (load1 ~33–40): dispersão visível
  nas rodadas (ex. ycsb_f_mc3 clean [0,255–0,399]); a leitura correta é
  a FRONTIER (par interno min-of-3) e as PHASE (medianas por commit),
  não os ratios absolutos.
