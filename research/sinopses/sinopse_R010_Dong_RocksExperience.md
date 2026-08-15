# Sinopse: R010 — RocksDB Experience

- **Ficha:** [`../fichamentos/ficha_R010_Dong_RocksExperience.md`](../fichamentos/ficha_R010_Dong_RocksExperience.md)
- **Tier:** D4
- **Tese (com locus):** Rocks é **library de um nó** (p. 2). O alvo de optimização migrou WA → **espaço** → CPU (abstract). “for most applications, space utilization was far more important than write amplification” (p. 5). 42 deploys: majority space-constrained (Fig. 3).
- **Número que importa:** Leveled WA 10–30, Table 3 WA 16 / space 10%; Dynamic Leveled ~13% vs estático até 25% (Table 4). Corrupção Rocks ~**1 / 3 meses / 100 PB**, 40% já nas réplicas (p. 9). 39 ZippyDB → **>25 configs** (Table 5). User timestamp **1.2–2.0×** vs ts na key (Table 6). WAL *off* só se a app já tem Paxos (p. 7). BlobDB = WiscKey (p. 6).
- **Para Pedra:** L9 confirmado (`src=ficha`). Espaço>WA confirma 0026-C / L17. L4 BlobDB confirma. **L25 MEASURE** WAL-skip só no apply Montanha (≠ L5b). **L26 MEASURE** checksum de ficheiro no copy. **L27 MEASURE** user timestamps (≠ L23). Não copiar a superfície de knobs.
- **Não ler isto como:** “desliguem o WAL do Pedra” (só com log de consenso da *app*) nem “WA 16× no lab de 8 MiB”.
