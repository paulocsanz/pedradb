# Sinopse: R017 — SILK

- **Ficha:** [`../fichamentos/ficha_R017_Balmau_SILK.md`](../fichamentos/ficha_R017_Balmau_SILK.md)
- **Tier:** D4
- **Tese (com locus):** p99 = interferência write/flush/compact, não WA (p. 753). Flush > L0→L1 > L≥1; BW no vale; preempt compacto alto (p. 758). Rate-limit e “adiar compact” adiam o spike (lições 2–3).
- **Número que importa:** Fig. 1 Rocks p99 **2–4 ordens** vs sem internos. Nutanix: SILK **2–3 ordens** melhor p99; **0 stall**; TRIAD flush atrasado **69×**; auto-tuned stalls **1%** do tempo. YCSB: throughput −≤7%; p99 write até 2 ordens. Pico 90% write degrada ~300 s. 200 MB/s disco, 1 GB RAM, WAL off.
- **Para Pedra:** `ConcurrentDb` já faz pipeline de flush; sem nomes de stall nem prioridade. **L41 MEASURE** — contadores + flush>compact. **Não** scheduler completo. Limiter só se `Env` o puder desligar. Teste curto mente. L3/L37 intactos (menos compact ≠ p99).
- **Não ler isto como:** “16 threads de compact” nem “desligar o WAL porque o paper mediu sem Clog”.
