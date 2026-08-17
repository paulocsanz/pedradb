# Next run — performance (0040 / 0039)

Handoff operacional: **[RFC-0040 § Next run](0040-fsync-always-beats-rocks-async.md#next-run-handoff)**.

**P1.1 + P1.2 feitos.** Apply MC mediana 0.93× async (p11); sticky sobe `avg_group` 1.1→1.55 e **não** fecha ≥1.0 de forma estável ([p12](../findings/rfc0040-p12/README.md)). Catch-up 200 µs piora qps.

Alvo de produto passou a **RFC-0041**: ≥ **2×** Rocks default em todas as shapes. P0.2 **done** — 0/16 ≥ 2.0 ([p02](../findings/rfc0041-p02/README.md)). idle64 **3/16** (C/E/scan) ([idle64](../findings/rfc0041-p11/idle64/README.md)). P1.1 **doing** — apply still pays a 64 MiB SST fsync; skip imm drain while writers are active.

**Peer oficial = Rocks default (`sync=false`).** Pedra mantém `fdatasync` e **mesmo assim** tem de ganhar. “Contrato diferente” não é desculpa nem critério de vitória. `SYNC=1` é coluna extra. Sem skip de sync, sem thread no core.
