# Sinopse: R006 — Monkey

- **Ficha:** [`../fichamentos/ficha_R006_Dayan_Monkey.md`](../fichamentos/ficha_R006_Dayan_Monkey.md)
- **Tier:** D4
- **Tese (com locus):** o I/O esperado de um point lookup zero-result é a **soma** dos FPRs. Bits uniformes não minimizam essa soma. “setting the false positive rate of each Bloom filter to be proportional to the number of entries in the run” (p. 2) — mais bits nos níveis **rasos**. Tira \(O(L)\) quando há ≳ 1.44 bits/entry (p. 8).
- **Número que importa:** fork LevelDB, **HDD 7200 RPM**, cache off, 5 bits/entry, \(T=2\): lookup **até 80%** mais baixo em zero-result à medida que \(N\) cresce (Fig. 11A, p. 10); **~30%** em non-zero (Fig. 11D); **~60%** menos memória para empatar LevelDB (Fig. 11C). “Navigable Monkey” (escolhe \(T\)+policy) **>2×** throughput — *não* é o ganho do Bloom sozinho (Fig. 11F, p. 12).
- **Para Pedra:** L1 `SHIP` (Bloom por SST = o SOTA de que o paper parte). L2 **continua `MEASURE`**: Pedra já está nos 10 bits/key com `MAX_LSM_LEVEL=3`. O 80% é seek de disco; o paper próprio relaxa o alvo de \(R\) duas ordens em flash (p. 10). Não ligar `with_monkey_bloom`. Se o MEASURE fechar: `bits_per_key` por SST via `t.len()` (App. C / Eqs. 17–18), teste de `get` miss sem false negative.
- **Não ler isto como:** “Monkey sempre 50–80% no Pedra” (HDD, cache off, zero-result, \(L\) grande) nem “2× throughput = Bloom” (isso é o navegador de \(T\)).
