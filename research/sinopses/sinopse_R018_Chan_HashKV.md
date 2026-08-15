# Sinopse: R018 — HashKV

- **Ficha:** [`../fichamentos/ficha_R018_Chan_HashKV.md`](../fichamentos/ficha_R018_Chan_HashKV.md)
- **Tier:** D4
- **Tese (com locus):** KV-separation circular devolve o WA no update Zipf porque o tail relocates cold-valid e o GC faz get na LSM (p. 1007, 1009). Remédio = `hash(key)` → grupo; validade = última append no grupo, sem get LSM (p. 1011).
- **Número que importa:** Fig. 2 Update: vLog **19.7×**, LevelDB 19.1×, Rocks **7.9×** (cache off). Exp 1 (cache on): HashKV **3.7–4.6×** vLog, write **−49.6%**. Abstract 4.6× / −53.4%. P0 hash é **7.9% mais lento** que vLog. Crash journal −6.5%. 256 B perde para LSM inline. RAID-0 6× Plextor, 40 GiB + 30%.
- **Para Pedra:** L4 confirmado (selective). L5b confirmado (não cache sem WAL). L5: tail é o desenho da Fig. 2. **L19 continua MEASURE** — 0026 escolheu C; 0028 P0 **não** começa. Precisa key no record + soak em que blobs/rewrite percam. L18: não SHIP tail como próximo GC.
- **Não ler isto como:** “implementem HashKV no lugar dos blobs” nem “19.7× é o nosso `baseline`”.
