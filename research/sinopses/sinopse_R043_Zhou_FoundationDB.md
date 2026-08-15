# Sinopse: R043 — FoundationDB

- **Ficha:** [`../fichamentos/ficha_R043_Zhou_FoundationDB.md`](../fichamentos/ficha_R043_Zhou_FoundationDB.md)
- **Tier:** D4
- **Tese (com locus):** KV ordered + TX **strictly serializable**; o resto é **layer** (p. 1–2). Engenharia: simulação determinística do binário *antes* da base (p. 2); falha do TS = reconfigurar, não mascarar com 2f+1 (p. 2–3).
- **Número que importa:** commit produção 22 / 281 ms (avg / p999); recovery mediana **3.08 s** (n=289); conflito 0.73%; CloudKit **0.5 M disk-years** sem corrupção. Janela MVCC **5 s**, TX **10 MB**. Lag SS←LS p999 médio 3.96 ms. Lab write 67→391 MBps (4→24 máquinas).
- **Para Pedra/Montanha:** L9 confirmado. **L28 MEASURE** aprofundar swarm/buggify no store (não reescrever em Flow). **L29 REFUSE** clonar Sequencer/Proxy/Resolver (já no RFC-0017). **L30 REFUSE** janela 5 s no kernel. Raft continua 2f+1. 2PC de intents ≠ FDB OCC.
- **Não ler isto como:** “Montanha deve ser FDB por dentro” nem “5 nines sem simulação do nosso binário”.
