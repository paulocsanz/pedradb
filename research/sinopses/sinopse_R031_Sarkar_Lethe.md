# Sinopse: R031 — Lethe

- **Ficha:** [`../fichamentos/ficha_R031_Sarkar_Lethe.md`](../fichamentos/ficha_R031_Sarkar_Lethe.md)
- **Tier:** D4
- **Tese (com locus):** persistência de delete é objectivo à parte (p. 893–894). Tombstone só morre no último nível; SoA é unbounded; full-tree é o anti-padrão (X-Engine p. 894). FADE = TTL exponencial + pick a_max/b (SO/SD/DD). KiWi = delete tiles (h); h=1 = clássico.
- **Número que importa:** §5.1 (1 GB, WAL off): space −48% (Dth=50%); compactos −45% (2% deletes); lookup **+17%**; Rocks deixa ~40% dos tombstones mais velhos que Dth; WA 1.4× → **+0.7%**. KiWi h=8, I/O −76%, hash 5× (1/7 do DB). Abstract 1.4× / 9.8× / +4–25% **não** reaparece no §5.
- **Para Pedra:** tombstone já existe; `compact_for_reads` = full-tree. **L38 MEASURE** FADE depois de L35, só com SLA Dth. **L43 REFUSE** KiWi (sem delete key; h>1 multiplica point I/O). Não filtrar delete com Bloom.
- **Não ler isto como:** “implementar Lethe” nem misturar o +18–35% da ficha R012 com o +0.7% daqui.
