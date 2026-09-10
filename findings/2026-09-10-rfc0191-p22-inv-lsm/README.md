# RFC-0191 P2.2 — Inv-LSM um passo (visible_at ∘ probe-order) + corolário R1

Data: 2026-09-10. Commit: (este land).

## O que subiu

- `formal/aeneas/lean/Merge.lean` (agora importa `ProbeOrder`):
  - Lema `inv_lsm_newest_first_never_non_live` ∀ newer older kind
    range_hidden: dado o probe-order newest-first do 0164
    (`first_probe_on_equal_lo newer older = ok newer`, ∀ — o kernel
    extraído é exatamente "devolve o mais novo"), se o filtro do path
    de get responde live (`merge.visible_at kind range_hidden = ok
    true`) então a versão é genuinamente viva: `kind = Value ∧
    range_hidden = false`. Abre `merge.visible_at` (o átomo R1
    registrado) e o def 0164; Deletion/RangeDeletion morrem no braço
    do kernel (ok false = ok true), Value escondido morre em
    `¬ range_hidden`.
  - Corolário `r1_get_never_returns_non_live` ∀ kind range_hidden: o
    filtro responde live ⇒ versão genuinamente viva. Braço Deletion
    fechado por P0.2 (`rw [r1_deletion_never_live] at hlive`),
    braço Value-escondido por P1.1 (`rw [(r1_get_atom _).2] at
    hlive`), braço RangeDeletion pelo lema (disjointness via
    `hkill.1`), braço honesto `exact inv_lsm_newest_first_never_non_live
    … hnew hlive` com `hnew := first_probe_on_equal_lo_newer` (o
    teorema do 0164). Citações não vacuas — cada uma fecha um braço.
- Linha de produto R1 **não** mudou (permanece `atom`); nenhum TSV
  tocado (sem linha nova em `close_proofs.tsv`; extract 276 intacto).

## Fronteiras (o que NÃO é)

- Um passo do primeiro-probe: `first_probe_on_equal_lo` é o
  tie-break do 0164 (equal-lo devolve o índice mais novo), não o walk
  inteiro (`probe_order_covering_loop` continua no kernel) — o lema
  cobre a primeira versão candidata que o get pode devolver, não
  toda a travessia multi-tabela.
- Não é `get` end-to-end (memtable + Níveis + cache seguem fora); é
  o átomo de liveness no path de get composto com a ordem de probe.
- `open` de dois kernels no mesmo ficheiro quebra: ambos definem
  `Slice.Insts.CoreCmpPartialOrdSlice` e o `ite` do merge colapsa —
  o def do probe entra qualificado
  (`pedra_aeneas_probe_order_kernel.first_probe_on_equal_lo`).

## Provas

- `lake build Merge` verde (1701 jobs).
- Gates no commit: product-floor GREEN (R1=atom …promoted=4≥4),
  depth-floor GREEN (extract=276 intacto), ledger GREEN (12
  ponteiros), zero `sorry` em `Merge.lean`.
