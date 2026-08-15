# Sinopse: R014 — Endure

- **Ficha:** [`../fichamentos/ficha_R014_Huynh_Endure.md`](../fichamentos/ficha_R014_Huynh_Endure.md)
- **Tier:** D4
- **Tese (com locus):** tuning *nominal* (point-optimal para um w) sobre-ajusta; deriva no mix (mesmo R/W, mais range) dá **2×** I/O (Fig. 1). Endure escolhe (T, bits Bloom, L|T) que maximiza o pior caso numa bola KL (p. 2–4). Retune online **inviável** (p. 2).
- **Número que importa:** modelo até **5×**; Rocks até **2.4×** / **90%** menos I/O. ρ=0 ≈ nominal; uniforme: nominal +5%. Table 3: robusto **sempre Leveling**; 10/15 ganhos, 2× −0.1. 8.6 M comparações, robusto >80%. Tuning <10 ms. ŵ≈w: **+20%** latência (Fig. 8).
- **Para Pedra:** sem knobs (T/Monkey/policy). **L12 MEASURE** — default estático robusto só se existirem ≥2 knobs. **Não** SHIP tuner. Table 3 **confirma** L3/L37 (não tiering/LL). L2/L36 intactos.
- **Não ler isto como:** “ponham SLSQP no `compact`” nem “5× no nosso `baseline`”.
