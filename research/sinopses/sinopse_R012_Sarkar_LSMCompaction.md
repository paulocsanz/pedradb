# Sinopse: R012 — LSM Compaction Design Space

- **Ficha:** [`../fichamentos/ficha_R012_Sarkar_LSMCompaction.md`](../fichamentos/ficha_R012_Sarkar_LSMCompaction.md)
- **Tier:** D4
- **Tese (com locus):** compactação = ponto em **4 primitivas** — trigger, data layout, granularity, data movement (p. 1–4). “There is no perfect compaction strategy” (p. 2).
- **Número que importa:** Rocks 6.11.4, t2 + io2 4000 IOPS, \(T=10\), 10 M × 128 B. Full move **63×** o ingest; Tier **23×**; partial **−34–56%** vs Full; tail write Tier **~25 ms** vs Old **1.3 ms**. Point: policy de pick não muda (TA III); Tier 1.1–2.2× pior. Universal perde >8 GB. TSD/TSA pagam +18–35% movimento para apagar mais cedo.
- **Para Pedra:** `compact_levels` = Full do par \(N\cup N+1\) → 1 SST, \(L\le 3\). **L35 MEASURE** file-granular + LO+1 (antes de Spooky). **L36 REFUSE** menu de 10 / auto-switch. **L37 REFUSE** universal/tiering default. **L38 MEASURE** TSD/TSA só com SLA de delete. L3 intacto.
- **Não ler isto como:** “implementem as 10” nem “tiering ganha writes” (o próprio paper recusa isso no tail, p. 12).
