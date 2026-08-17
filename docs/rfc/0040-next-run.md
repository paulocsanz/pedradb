# Next run — performance (0040 / 0039)

Handoff operacional: **[RFC-0040 § Next run](0040-fsync-always-beats-rocks-async.md#next-run-handoff)**.

**P1.1 + P1.2 feitos.** Apply MC mediana 0.93× async (p11); sticky sobe `avg_group` 1.1→1.55 e **não** fecha ≥1.0 de forma estável ([p12](../findings/rfc0040-p12/README.md)). Catch-up 200 µs piora qps.

Alvo de produto passou a **RFC-0041**: ≥ **2×** Rocks default. scansst **1/16** (MVCC **2.58**; scan **1.15**). idleinc/b4c1 rejected (scan 0.24/0.28). FLOOR off. 1c A still 1/`fdatasync` (p50 23.3 µs).

**Peer oficial = Rocks default (`sync=false`).** Pedra mantém `fdatasync` e **mesmo assim** tem de ganhar. “Contrato diferente” não é desculpa nem critério de vitória. `SYNC=1` é coluna extra. Sem skip de sync, sem thread no core.
