# RFC-0212 P2.1 — átomo `catalog:prepare_error_aborts_earlier` (cadência txn 4/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem prepare_error_aborts_earlier_fate_iff :
    ∀ (v : Bool), (prepare_error_aborts_earlier = ok v) ↔ v = true
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F50: um passo de prepare que falha ABORTA os intents já duráveis
em ranges preparados anteriormente — o fate da limpeza é
exatamente `true`.

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn prepare_error_aborts_earlier() -> bool {
    true
}
```

## AS-IS recusado (a mentira)

```rust
pub fn prepare_error_aborts_earlier_as_is() -> bool {
    false
}
```

Volta no `?` do `NotLeader` sem limpeza: intents preparados antes
da falha ficam para sempre como `Conflict` nas chaves. A planta
DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- prepare_error_aborts_earlier_on_live_queued_is_not_ok
test three_teeth_queued::prepare_error_aborts_earlier_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

## Cirurgia de catálogo

`promote_atom.py prepare_error_aborts_earlier
prepare_error_aborts_earlier_fate_iff ...`: floor_atom 94→95,
floor_extract 184→183; linha `atom` no `close_proofs.tsv` (entry
`prepare_error_aborts_earlier`); catálogo `data_fate` removido
com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 30→29, floor_atom 94→95, floor_extract 184→183.

## Gates

`check_depth_floor.py` GREEN: extract=183 (floor 183), atom=95
(floor 95), data_fate=29<=29, residuals == live.
