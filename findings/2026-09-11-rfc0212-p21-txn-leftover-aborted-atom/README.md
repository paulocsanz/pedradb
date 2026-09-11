# RFC-0212 P2.1 — átomo `catalog:leftover_txn_is_aborted` (cadência txn 2/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem leftover_txn_is_aborted_fate_iff :
    ∀ (v : Bool), (leftover_txn_is_aborted = ok v) ↔ v = true
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F35: TX preparada sobrante (crash sem log do coordenador) é
ABORTADA no recovery — o fate do sobrante não cometido é
exatamente `true` (via `leftover_fate(false)`).

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn leftover_txn_is_aborted() -> bool {
    leftover_fate(false)
}
```

## AS-IS recusado (a mentira)

```rust
pub fn leftover_txn_is_aborted_as_is() -> bool {
    leftover_fate_as_is(false)
}
```

Mantém os intents vivos: `Conflict` imortal para as chaves
preparadas e reuso de id de transação. A planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- leftover_txn_is_aborted_on_live_reopen_is_not_ok
test three_teeth_queued::leftover_txn_is_aborted_on_live_reopen_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

## Cirurgia de catálogo

`promote_atom.py leftover_txn_is_aborted
leftover_txn_is_aborted_fate_iff ...`: floor_atom 92→93,
floor_extract 186→185; linha `atom` no `close_proofs.tsv` (entry
`leftover_txn_is_aborted`); catálogo `data_fate` removido com
`atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 32→31, floor_atom 92→93, floor_extract 186→185.

## Gates

`check_depth_floor.py` GREEN: extract=185 (floor 185), atom=93
(floor 93), data_fate=31<=31, residuals == live.
