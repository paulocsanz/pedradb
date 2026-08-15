# Sinopse: R041 — Percolator

- **Ficha:** [`../fichamentos/ficha_R041_Peng_Percolator.md`](../fichamentos/ficha_R041_Peng_Percolator.md)
- **Tier:** D4
- **Tese (com locus):** SI multi-row *em cima* de um KV que só é atómico por row: versões no timestamp, locks/`write` persistentes na mesma célula, 2PC **cliente** com primary lock, oracle monotónico (p. 3–6, Figs. 4–6). Observers disparam **outra** TX — não mantêm invariante (p. 6).
- **Número que importa:** Caffeine vs MR: mediana **>100×**, idade do resultado **−50%**, ~2× recursos (p. 1, 9). Fig. 8 (1 tablet): write **0.23×** Bigtable (~4× overhead). TPC-E-like: 2–5 s, **~30× CPU** vs DBMS 64-core, linear até 15k cores / 11 200 tps (p. 11). Não é OLTP.
- **Para Pedra:** L22 `REFUSE` o protocolo de colunas+2PC cliente (kernel já é OCC+WAL; store já é intents FDB). L23 `REFUSE` oracle-serviço (sequence / `snapshot_begin`). OCC do kernel é **mais forte** que SI: valida read-set (corta write skew na key lida). Observers ≠ Watch ≠ Fold.
- **Não ler isto como:** “Montanha deve ser Percolator” (2PC deles é outra geometria) nem “4×/30× aplica-se ao put local”.
