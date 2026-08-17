# Next run — performance (0040 / 0039)

Handoff operacional: **[RFC-0040 § Next run](0040-fsync-always-beats-rocks-async.md#next-run-handoff)**.

**P1.1 + P1.2 feitos.** Apply MC mediana 0.93× async (p11); sticky sobe `avg_group` 1.1→1.55 e **não** fecha ≥1.0 de forma estável ([p12](../findings/rfc0040-p12/README.md)). Catch-up 200 µs piora qps.

Fazer a seguir: **P2.1 scan** (p50 já ganha do async; qps morre com L0=4–9). Depois P2.2 pipeline encode∥fsync no host se apply MC ainda for o alvo.

**Peer oficial = Rocks default (`sync=false`).** Pedra mantém `fdatasync` e **mesmo assim** tem de ganhar. “Contrato diferente” não é desculpa nem critério de vitória. `SYNC=1` é coluna extra. Sem skip de sync, sem thread no core.
