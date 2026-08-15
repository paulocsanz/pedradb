# Sinopse: R045 — CockroachDB

- **Ficha:** [`../fichamentos/ficha_R045_Taft_CockroachDB.md`](../fichamentos/ficha_R045_Taft_CockroachDB.md)
- **Tier:** D4
- **Tese (com locus):** SQL conversacional + serializable + Multi-Raft por Range ~64 MiB + HLC 500 ms, **sem** TrueTime (p. 1–3, 7). Não é strict serializable (p. 7). SQL está *no* binário — contrário de L9.
- **Número que importa:** TPC-C 100k warehouses / 8 TB / **1.25 M tpmC** / 98.8% / 81 nós / p90 486 ms. Parallel Commits +72% / −47% p50 (Fig. 2). Replicação −48% (RF=3). Follower reads ~2 s. Lease heartbeat 4.5 s.
- **Para Pedra/Montanha:** L9 confirmado (não clonar o produto). L29 confirmado (Multi-Raft fica vs Sequencer). **L39 MEASURE** split/merge + leaseholder no store quando houver >1 grupo. **L40 REFUSE** HLC/intents-CRDB/Parallel Commits no kernel. L24 intacto (closed ts ≠ fold). Pebble **não** está no eval (Rocks black box).
- **Não ler isto como:** “Montanha deve ser CRDB” nem “Pebble é o motor deste paper”.
