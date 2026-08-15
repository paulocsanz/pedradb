# Sinopse: R016 — ADOC

- **Ficha:** [`../fichamentos/ficha_R016_Yu_ADOC.md`](../fichamentos/ficha_R016_Yu_ADOC.md)
- **Tier:** D4
- **Tese (com locus):** stall = data overflow, não uma causa (CPU/BW/L0/fundo). MMO/L0O/RDO = MT/L0/PS (p. 65, 70). Tuner online de threads + batch, AIMD, Tw=1 s (p. 72). SILK é complementar (p99), não o mesmo eixo.
- **Número que importa:** Fig. 12 fillrandom: ADOC vs SILK-O +66.7/37.8/31.0/55.1%; vs Rocks-AT no PM 211.5/50.0 ≈ +323% (abstract 322.8%). Stall vs SILK-O −45.2/−8.7/−10.3 / NVMe **+1.5%**. p99 vs SILK-O +70.1/+131.2/+242.9% fora do PM. SILK-O até +76.8% RAM; intro +22.2% média. 87.9% só no abstract. SILK 5.7.1 vs ADOC 7.5.3; SILK-O é 8 thr / 512 MB *manual*.
- **Para Pedra:** taxonomia entra em **L41 MEASURE** (nomes). **L42 REFUSE** tuner — 0 knobs vivos, DST-hostil. Não ligar L0-stop/PS. Não 16 threads. L3/L37/L36 intactos.
- **Não ler isto como:** “66% melhor que SILK” sem a Fig. 16 (p99) nem a mistura de versões.
