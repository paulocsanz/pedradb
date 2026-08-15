# Sinopse: R048 — TiDB

- **Ficha:** [`../fichamentos/ficha_R048_Huang_TiDB.md`](../fichamentos/ficha_R048_Huang_TiDB.md)
- **Tier:** D4
- **Tese (com locus):** isolation HTAP ≠ mais followers. Learner **não** vota nem entra no quorum; replica async; consistência no **read** (p. 2). TiFlash ainda manda **read-index ao leader** (§4.2.4). Freshness medida ≈ **1 s** (Table 4), não µs. Isolation de TPS ≤10% (Fig. 10) vs MemSQL **>5×** (Fig. 12) é hardware separado.
- **Número que importa:** 100 W + 32 AC: 84–87% dos lags < 1 s; só ~49% < 100 ms. HyPer/HANA no mesmo servidor: −5×/−3× (p. 2). 2PC = Percolator no SQL engine (p. 3).
- **Para Pedra:** L8 `SHIP` confirmado — fold não é voter. **L24 `REFUSE`** read-index/SI no fold (remete o request path ao SoR; RFC-0024 proíbe). TiFlash ≠ fold: partilham o papel Raft, não o formato nem a semântica de read. L7/L22/L23 intactos.
- **Não ler isto como:** “fold = TiFlash” nem “learner read é LocalApplied”.
