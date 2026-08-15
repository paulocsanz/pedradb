# Sinopse: R013 — Spooky

- **Ficha:** [`../fichamentos/ficha_R013_Dayan_Spooky.md`](../fichamentos/ficha_R013_Dayan_Spooky.md)
- **Tier:** D4
- **Tese (com locus):** Full Merge gasta disco (transient SA ~2× no \(L\), utilização ≤50%); Partial gasta WA (borda + GC SSD). Spooky alinha fronteiras ao \(L\) e faz merge de um grupo (p. 1, 5–8).
- **Número que importa:** NVMe 960 GB, Rocks, \(T=5\), 16 bg. Lógico Partial/Spooky **644 GB** vs Full **369 GB**. Total WA **≈2.5×** melhor que Partial; GC **>2×** mais barato. Writes uniforme 54k vs Partial 26k vs Full 87k (*menos dados*). \(X=L-2\); \(X=L-1\) e \(L-4\) crasham.
- **Para Pedra:** `compact_levels` = Full-do-par, \(L\le 3\). **L35 primeiro.** **L11 MEASURE** src=ficha — só depois de ficheiro-granular *e* se utilização SSD / \(L\ge 4\) for o problema. Transient SA do Pedra **não** é 50% do dataset. L3/L36/L37 intactos. Não implementar.
- **Não ler isto como:** “o próximo patch é Spooky” nem “Full do Pedra desperdiça metade do disco”.
