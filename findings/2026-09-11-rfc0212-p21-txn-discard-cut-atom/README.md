# RFC-0212 P2.1 — átomo `catalog:discard_cut` (cadência txn 1/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem discard_cut_fate_iff :
    ∀ (from_index commit v : U64),
      (discard_cut from_index commit = ok v) ↔
        ((v = core.num.U64.saturating_add commit 1#u64
            ∧ from_index < core.num.U64.saturating_add commit 1#u64)
          ∨ (v = from_index
            ∧ ¬ (from_index < core.num.U64.saturating_add commit 1#u64)))
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F47: o corte do discard é EXATAMENTE `max(from_index, commit+1)` — o
menor índice descartável nunca fica em ou abaixo do commit; índice
cometido sobrevive sempre, índice acima do commit corta em `from`.

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn discard_cut(from_index: u64, commit: u64) -> u64 {
    from_index.max(commit.saturating_add(1))
}
```

## AS-IS recusado (a mentira)

```rust
pub fn discard_cut_as_is(from_index: u64, _commit: u64) -> u64 {
    from_index
}
```

Corta em `from_index` mesmo com commit acima: dado cometido é
descartado do log. A planta DST abaixo refuta no cluster vivo.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- discard_cut_on_live_queued_is_not_ok
test three_teeth_queued::discard_cut_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

Técnica: `broadcast_append` de um `TxnPrepare` que permanece no log
RAM do líder no índice de commit (o prefixo aplicado é compactado
fora — F27); `discard_uncommitted_from(1, leader, cut_from)` com
`cut_from = commit`: a produção (`cut = commit+1`) mantém a entrada;
o as-is a deletaria.

## Wrapper inscrito no gate

`StoreTxn` entra no array LIBS do `lean_extracts.sh` (62 libs) neste
commit — buraco pré-existente desde o RFC-0191: o wrapper tinha
teoremas mas nenhum gate o compilava (nada o importava). Fechado.

## Cirurgia de catálogo

`promote_atom.py discard_cut discard_cut_fate_iff ...`:
floor_atom 91→92, floor_extract 187→186; linha `atom` no
`close_proofs.tsv` (entry `discard_cut`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 33→32, floor_atom 91→92, floor_extract 187→186.

## Gates

`check_depth_floor.py` GREEN: extract=186 (floor 186), atom=92
(floor 92), data_fate=32<=32, residuals == live.
