# LEDGER — o que a literatura faz ao Pedra

**Fonte única de verdade** das decisões *depois* (ou explicitamente *antes*,
como hipótese) de uma ficha. Atualizar **na mesma sessão**. Chat / `/tmp`
não conta.

Espelho do ledger DST (`../determinismo/pedradb-dst/findings/LEDGER.md`):
tentou / vale / falhou / bloqueado / falso. Aqui o objeto é **estratégia
de paper**, não bug.

Norma: [`QUALIDADE.md`](QUALIDADE.md). Uma linha `SHIP` sem ficha D3 é
hipótese — marcar `src=listed`.

## Tags

| Tag | Significado |
|-----|-------------|
| `SHIP` | Está no código, ou a ficha D3 diz para implementar agora |
| `MEASURE` | Hipótese mensurável; precisa bench/DST nomeado |
| `REFUSE` | Não implementar (razão + paper). Recusar é resultado |
| `OPEN` | Buraco conhecido (ex. vlog GC) |
| `BLOCKED` | Sem PDF legal / paywall / hardware que não temos |
| `FALSE` | O claim não se aplica ao nosso setting (ficha explica) |
| `SUPERSEDED` | Substituído por ficha/RFC mais novo |

`src`: `ficha` · `rfc` · `listed` · `incumbent` · `bench`

## A. Decisões já no repo (antes deste catálogo)

Vêm de RFCs + PDFs em `docs/references/`. Promover/rebaixar quando a
ficha D3 correspondente existir.

| ID | Decisão | Tag | src | Papers | Onde |
|----|---------|-----|-----|--------|------|
| L1 | Bloom por SST (skip table) | `SHIP` | rfc | R006 (alocação Monkey **não**) | RFC-0014 |
| L2 | Alocação Monkey de FPR | `MEASURE` | rfc | R006 | RFC-0012 |
| L3 | Lazy Leveling | `REFUSE` | rfc | R007 | RFC-0012 — reabrir só com `baseline` |
| L4 | Vlog threshold spill (não *todos* os values) | `SHIP` | ficha | R005 | RFC-0014 P2.2; ficha p. 5/9–10 — ganho some se value ≲ key |
| L5a | Vlog GC full-rewrite (`compact_vlog`) | `SHIP` | rfc | R005 | RFC-0016 P0.1; **não** é o GC do paper |
| L5 | Vlog GC incremental (head/tail + punch) | `OPEN` | ficha | R005, R018 | WiscKey §3.3.2; Pedra não guarda key no record do vLog |
| L5b | Dropar WAL da LSM porque o vLog tem keys | `REFUSE` | ficha | R005 | §3.4.2; vLog Pedra é `len\|crc\|data` — sem key no record |
| L6 | MemTable skiplist/arena | `REFUSE` | rfc | R097 | keep `BTreeMap` até write path CPU-bound |
| L7 | Coluna no mesmo MANIFEST | `REFUSE` | rfc | R032, R066 | HTAP note §0 |
| L8 | Fold = LocalApplied, cursor após apply | `SHIP` | rfc | R096, R090 | RFC-0024 |
| L9 | Pedra é local; produtos são camadas | `SHIP` | rfc | R043, R044 | RFC-0010 / grail |
| L10 | Hardware PIM/CSD/FPGA como dep | `REFUSE` | listed | R071–R073 | P0 software |

## B. Aberto pelo catálogo (ainda `listed` — não é prova)

| ID | Decisão candidata | Tag | src | Papers | Fecha se |
|----|-------------------|-----|-----|--------|----------|
| L11 | Compact granulado (Spooky) | `MEASURE` | listed | R013 | ficha D3 + file-granular compact existe |
| L12 | Endure / robust vs point-optimal | `MEASURE` | listed | R014, R015 | ficha + um knobs doc |
| L13 | Disco / REMIX range index | `MEASURE` | listed | R022, R023 | `scan` for um problema |
| L14 | Splinter/Turtle em vez de LSM | `MEASURE` | listed | R029, R040 | perder workload nomeado por &gt;2× |
| L15 | Learned index no SST | `REFUSE` | listed | R019, R098 | só se Bourbon ficha D3 disser o contrário |
| L16 | Compact-as-a-service | `REFUSE` | listed | R059 | local compact chato e correto |

## C. Falsos / bloqueados

_Nenhum ainda. Uma linha `FALSE` exige ficha que mostre o mismatch
(hardware, Rocks version, workload)._

## Como acrescentar

1. Ficha ≥ D3 (ou `src=listed` consciente, no bloco B).
2. Uma linha aqui, mesmo id estável (`Lnn`).
3. Se contradisser RFC-0012/0014, editar o RFC **na mesma mudança**.
