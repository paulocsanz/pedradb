# Correlação: value-store GC (R005 + R018 + Pedra)

**Status:** cruza fichas D4 (R005, R018) com o código actual.
Hipótese vs. verificado marcado.

## Escopo

Como o Pedra deve evoluir o value store depois de WiscKey e HashKV.
RFCs: 0026–0029.

## O que está verificado

| Fonte | Claim | Locus | Pedra |
|-------|-------|-------|-------|
| R005 D4 | Compact só precisa ordenar keys; value no log | p. 1, 5 | L4 spill — já |
| R005 D4 | Delete não toca o vLog | p. 5 | verdadeiro hoje |
| R005 D4 | GC online precisa de `(key,value)` no record + head/tail | p. 6–7 | **não** temos; rewrite em vez disso |
| R005 D4 | Range de values pequenos em log random-fill **perde** 12× | p. 10 | scan prefetch N=4 (0029 P0.3) |
| R018 D4 | vLog circular no Update Zipf: WA **19.7×** (Rocks 7.9×) | p. 1009 Fig. 2 | risco do tail; 0026 8 MiB **não** reproduziu |
| R018 D4 | validade = última escrita no grupo; sem get LSM | p. 1011 | ideia de 0028; L19 MEASURE |
| R018 D4 | HashKV 3.7–4.6× vLog; write −49.6%; P0 hash −7.9% vs vLog | p. 1014 | RAID-0 + cache; não o nosso `baseline` |
| R018 D4 | 256 B: sep perde para LSM inline | p. 1015 | confirma L4 |
| Código | `compact_vlog` / `compact_blob` = rewrite + remap | `vlog.rs` | L5a / 0029 P0 SHIP |

## Hipóteses (não confirmar como fato Pedra)

- Abstract 4.6× / −53.4% — hardware RAID-0, LevelDB 1.20, 40 GiB, 30% reserve, crash OFF.
- Titan/BlobDB “discardable ratio” — sem ficha.
- Prefetch N=32 (WiscKey) — 0029 usa N=4.

## Tensões

| Se escolhermos | Compramos | Pagamos |
|----------------|-----------|---------|
| 0027 tail | menor delta no ficheiro de hoje | WA de update (Fig. 2); precisa key no record |
| 0028 hash | GC O(grupo quente); sem get LSM | writes espalhados; segment table; GC journal |
| 0029 blobs | drop de ficheiro como SST; prefetch | muitos ficheiros; hot key suja N blobs |
| ficar no rewrite | crash story já DST’d | custo O(live) sempre |

Fecha: **0026-C** + **0029 P0** no código. HashKV D4 **não** reabre C.
L19 só se um soak nomeado perder.

## Ledger

L4/L5b `SHIP`/`REFUSE` confirmados por R018. L5 `OPEN`. L19 `MEASURE`
src=ficha. L17/L20 blobs. L18: não SHIP tail como próximo GC.

## Ressalvas

- WiscKey YCSB “ganha as 6” **não** inclui o scan de 4 GB / 64 B (ficha R005).
- HashKV crash = injection, não DST.
