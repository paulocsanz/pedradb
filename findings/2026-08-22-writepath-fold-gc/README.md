# 2026-08-22 — Write path: parked-fold O(n²) + versões ilimitadas (fix: VecDeque + GC no fold)

Commit base do perfil: `54d0b24`. Fix em cima (VecDeque de versões + absorb
oldest-first + GC de versões por floor de snapshot-list; compat ON, core OFF).

## Sintoma

- Shapes de escrita (a/f) abaixo do alvo ≥5× vs RocksDB default
  (`ROCKS_PARITY_SYNC=0`), com oscilação entre rodadas.
- Um núcleo inteiro queimado pelo compact worker (`spawn_compact_worker` →
  `fold_parked_once_off_lock`) durante ycsb_a.
- RSS ~10,1 GB (pico de footprint 13,2 GB) para ~100 MB de dados vivos em
  20M ops zipfian sobre 1024 chaves.

## Perfil (sample, ycsb_a timed loop, 2715 amostras)

| Componente | Samples (self) | Nota |
|---|---|---|
| `Engine::put` | 46% | loop de escrita |
| `get_probe` | 33% | lado leitura (cache P1.3 já tratado) |
| WAL (submit_inner) | ~444 total | write syscall 285 + read_exact_into 43 + CRC 68 + encode 15 + memmove/memset 33 |
| memtable | ~286 total | BTreeMap 47 + memcmp 150 + apply 49 + free 40 |
| publish/invalidações | ~122 | |
| commit self | ~73 | |

Achado decisivo FORA do loop: `fold_parked_once_off_lock` (clone+absorb) com
6 533 de 6 588 samples dentro de `insert_map` → memmove — o compact worker
queimava 100% de um núcleo só em memmove de `Vec::insert(0)`.

## Causa-raiz (dupla)

1. **`Versions::Many(Vec<Version>)` com `Vec::insert(0)`** por versão mais nova
   em chave quente: cada insert no índice 0 desloca o vetor inteiro — O(n²)
   de memmove no fold do park (zipfian = chaves quentes com milhares de
   versões).
2. **Nenhuma coleta de versões**: a lista por chave cresce sem limite enquanto
   o park não flui — ~100 MB vivos viram 15,7 GB de RSS; o fold então paga o
   quadrático de uma vez.

## Fix

1. `Versions::Many(VecDeque<Version>)`; inserção por busca binária no deque
   (`ver_cmp` invertido, listas newest-first).
2. `absorb_with_floor` consome as versões do **outro mais velho primeiro**
   (`vs.into_iter().rev()`): toda inserção cai no índice 0 — O(1) amortizado.
   (Iterar newest-first re-introduz O(k) de deslocamento no índice crescente
   k: microbench do merge 292 ms → ~2 ms.)
3. **GC de versões no fold** com floor = min(visible_sequence,
   oldest_pinned_sequence, menor bound no registry de OCC): por chave mantém
   `{seq > floor} ∪ {mais nova ≤ floor}` + irmãs de mesmo seq; colapsa
   `Many` de 1 elemento de volta a `One`. Watermark ratcheta in-memory
   (`raise_earliest_readable`; replay do WAL restaura as versões no reopen).
   OFF por default no core (F20 intacto), **ON no compat** (paridade
   rust-rocksdb: supersede abaixo do snapshot mais velho cai). Transações OCC
   registram bound publicado **antes** de ler o snapshot — fold que não viu
   a entrada não pode ter floor acima dela.

Contratos verificados: `snapshot()` do compat pinna (C5/C6); respostas do
point-cache seguram clones de `Bytes` (valores já resolvidos); lazy feed =
last-per-key (a mais nova sempre fica); scans sob read lock;
`compact_reclaim` usa a mesma semântica de floor.

## A/B (mesma caixa, suíte ycsb completa por rodada, 20M ops, zipfian 1024 chaves, 3 rodadas alternadas)

old = `54d0b24`; new = fix (árvore = old + fix; binário único por braço,
build 12:11, inalterado durante todo o A/B).

| shape | old med qps | new med qps | delta med | por rodada |
|---|---|---|---|---|
| ycsb_a | 70 159 | 65 447 | −6,7% | −12,0 / −6,7 / **+12,1** |
| ycsb_b | 595 959 | 684 026 | **+14,8%** | +23,4 / −0,5 / +14,8 |
| ycsb_c | 9 335 634 | 9 644 155 | +3,3% | +4,9 / +5,4 / −0,6 |
| ycsb_d | 600 907 | 701 750 | **+16,8%** | +51,8 / +2,2 / +16,8 |
| ycsb_e | 598 262 | 557 139 | −6,9% | −11,3 / −7,4 / **+12,9** |
| ycsb_f | 67 306 | 72 098 | **+7,1%** | +2,3 / +11,4 / +39,1 |

| recurso | old | new | delta |
|---|---|---|---|
| CPU user (suíte inteira) | 1 073–1 109 s | 70,8–71,6 s | **−93%** |
| pico de RSS | 14,8–15,9 GB | 5,9–6,2 GB | **−62%** |
| wall da suíte (soma das shapes) | 672–811 s | 653–687 s | −9 a −20% |
| tamanho do diretório do DB | 5,3 GB | 1,9 GB | −64% |

Leitura: `a`/`e` são limitados pela latência de fdatasync e dominados por
drift da caixa (o próprio old cai 72,6k→65,0k entre r1 e r3; o new r3 é o
maior de todos os 6 runs: 72,9k). `b`/`d`/`f`/`c` melhoram consistente ou
ficam planos; o núcleo queimado do fold (−93% CPU user) e o RSS some. O
GC também encolhe o DB em disco (versões superseded não são re-flushadas).

Nota: o binário "new" inclui também a wave 3 do RFC-0048 (F185–F195,
commitada em `f1acb4f`) — o discriminador de 3 binários
(old / só-fix / combo) em 5M ops separa as contribuições; tabela no
apêndice quando coletada.

## Testes

- `memtable`: `gc_floor_keeps_exact_reads_at_or_above`,
  `gc_floor_none_keeps_everything`, `gc_collapses_pair_back_to_one`,
  `absorb_with_floor_merges_and_gcs`,
  `hot_key_overwrite_fold_is_not_quadratic` (2×20k versões, absorb < 250 ms).
- `concurrent`: `fold_without_gc_keeps_every_version`,
  `fold_with_gc_collapses_below_floor`, `fold_with_gc_respects_open_pin`,
  `fold_with_gc_respects_open_occ_transaction`.
- `compat`: `fold_gc_keeps_pinned_snapshot_and_bounds_versions`.
- Árvore combinada (fix + wave 3): core 390/0, compat 47/0, ops 9/9, sim 42/42.
