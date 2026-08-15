# Sinopse: R005 — WiscKey

- **Ficha:** [`../fichamentos/ficha_R005_Lu_WiscKey.md`](../fichamentos/ficha_R005_Lu_WiscKey.md)
- **Tier:** D4
- **Tese (com locus):** Compact LSM só precisa ordenar **keys**. Values num log à parte cortam o rewrite. “only keys are kept sorted in the LSM-tree, while values are stored separately in a log” (p. 1).
- **Número que importa:** random-load 100 GB, key 16 B: **46×** (value 1 KB) e **111×** (4 KB) vs LevelDB 1.18; WA efetiva ≈ **1.14** na fórmula 16 B+1 KB (p. 5, 9). O paper **perde 12×** no range de 4 GB sobre random-fill de 64 B (p. 10). Hardware: Samsung 840 EVO, 64 GB RAM, compressão off.
- **Para Pedra:** L4 `SHIP` (spill por threshold — recorte certo: o ganho some em value ≲ key). L5 cisão: rewrite `compact_vlog` já existe; GC incremental head/tail + hole-punch continua `OPEN`. **Não** dropar o WAL (§3.4.2): o vLog Pedra não guarda a key no record.
- **Não ler isto como:** “WiscKey sempre ganha” (eles escrevem o contrário, p. 2 e p. 10) nem “WA 50× no load de 100 GB” (Fig. 2 mede **14**).
