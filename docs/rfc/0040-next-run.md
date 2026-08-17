# Next run — performance (0040 / 0039)

Handoff operacional: **[RFC-0040 § Next run](0040-fsync-always-beats-rocks-async.md#next-run-handoff)**.

**P1.1 feito** (`cbb000f`+este commit): `run_deps_clients` + 3 runs em [findings/rfc0040-p11](../findings/rfc0040-p11/README.md). Apply MC4 mediana **0.93×** Rocks async; raftlog MC run1 **1.57×** (mediana 0.61, ruído).

Fazer a seguir: **P1.2** só se quiser fechar apply MC 0.93→≥1.0 (catch-up / group size). Senão **P2.1** scan (p50 já ganha; qps cai na cauda L0). Script: `scripts/tikv_ycsb_parity_mc_async.sh`.

Não reabrir: fd always-on *pode* ganhar do async (piso = ack/grupo). Coluna async obrigatória. Sem skip de sync, sem thread no core.
