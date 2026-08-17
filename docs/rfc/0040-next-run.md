# Next run — performance (0040 / 0039)

Handoff operacional: **[RFC-0040 § Next run](0040-fsync-always-beats-rocks-async.md#next-run-handoff)**.

Fazer primeiro: **P1.1** — harness MC apply+raftlog + remesura Pedra · Rocks sync · Rocks **async** (`ROCKS_PARITY_CLIENTS=4`). O `run_clients` de hoje **não** cobre apply/raftlog.

Não reabrir: fd always-on *pode* ganhar do async (piso = ack/grupo). Coluna async obrigatória. Sem skip de sync, sem thread no core.
