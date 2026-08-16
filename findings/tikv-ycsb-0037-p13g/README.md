# RFC-0037 P1.3 — deps_scan fixe (11/11 gate verde)

p13g (official): 11/11 `meets_floor` a 0.5, min 0.578.
deps_scan 177 073 vs Rocks 290 058 (clean) = 0.610. Antes (p21d): 131 927 / 291 531 = 0.453.
Rocks apply dessa run estava deprimido (1 877 vs banda limpa 5.1–5.6k); normalizando apply
ao limpo: 2 959 / 5 620 = 0.53 ≥ 0.5.

Três mudanças (P1.3):
1. `SstRangeIter` termina na 1.ª chave > end (fim do tail-walk do bloco).
2. `AnswerCache`: FIFO O(1) + FxHash + chave inline no stack (era LRU com scan
   O(capacity) por insert + malloc + SipHash por op — 100% miss no deps_scan).
3. `count_in_range` por referência (`count_visible`): merge sem clone por entrada
   (janelas de count varrem todas as versões MVCC).

Repetições do mesmo binário: p13e 0.610 (Rocks 287k), p13f 0.667 (Rocks 291k),
p13c 0.559 (Rocks 286k). WAL fdatasync before Ok. Único cliente, `Mutex<Db>` no compat.
