# Sinopse: R007 — Dostoevsky

- **Ficha:** [`../fichamentos/ficha_R007_Dayan_Dostoevsky.md`](../fichamentos/ficha_R007_Dayan_Dostoevsky.md)
- **Tier:** D4
- **Tese (com locus):** com Blooms Monkey, point / long-range / space-amp vivem no nível \(L\); o update paga **todos** os níveis. Merges em 1…\(L-1\) são “superfluous” (p. 1). Lazy Leveling = tiering em cima + leveling em \(L\) (p. 6). Point e space iguais a leveling; update \(O((L+T)/B)\); short range **pior**.
- **Número que importa:** RocksDB fork, **RAID 7200 RPM**, 1 TB × 1 KB, cache 10%, 10 bits/entry. Fig. 10A: Dostoevsky ≈ 1.0; Monkey ≈ 0.85–0.95; Rocks default ≈ 0.3–0.45 (normalizado). Fig. 10B: **nenhuma** policy fixa ganha em todo o eixo. Space-amp de LL sobe com leveling (Fig. 10F). “Strictly dominates” é o *navegador* \((T,K,Z)\), e os pontos Dostoevsky do §5 foram **pré-sintonizados** (App. F) — não medem transição.
- **Para Pedra:** L3 **continua `REFUSE`** (agora `src=ficha`). LL exige L2 (Eq. 3); Pedra tem `MAX_LSM_LEVEL=3` e `scan` first-class. Reabrir só com L2 shipped + write-amp num workload nomeado com \(L\ge 5\) + scan não bound + DST da transição.
- **Não ler isto como:** “ponham Lazy Leveling no default” (p. 8: *no single policy*) nem “2–3× Rocks” (HDD, normalizado, navegador ≠ LL).
