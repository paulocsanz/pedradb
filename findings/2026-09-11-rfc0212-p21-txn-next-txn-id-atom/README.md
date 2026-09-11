# RFC-0212 P2.1 — átomo `catalog:next_txn_id_after` (cadência txn 3/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem next_txn_id_after_fate_iff :
    ∀ (max_seen v : U64),
      (next_txn_id_after max_seen = ok v) ↔
        ((v = 1#u64
            ∧ core.num.U64.saturating_add max_seen 1#u64 < 1#u64)
          ∨ (v = core.num.U64.saturating_add max_seen 1#u64
            ∧ ¬ (core.num.U64.saturating_add max_seen 1#u64 < 1#u64)))
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F35: o próximo id de transação é EXATAMENTE `max(max_seen+1, 1)` —
nunca reusa id ainda em disco ou no contador durável; o piso 1 só
vence quando `max_seen+1` satura abaixo de 1 (U64 em 0).

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn next_txn_id_after(max_seen: u64) -> u64 {
    max_seen.saturating_add(1).max(1)
}
```

## AS-IS recusado (a mentira)

```rust
pub fn next_txn_id_as_is(_max_seen: u64) -> u64 {
    1
}
```

Reinicia o contador em 1: id de transação viva é reusado — colide
com intents sobreviventes. A planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- next_txn_id_after_on_live_reopen_is_not_ok
test three_teeth_queued::next_txn_id_after_on_live_reopen_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

## Cirurgia de catálogo

`promote_atom.py next_txn_id_after next_txn_id_after_fate_iff ...`:
floor_atom 93→94, floor_extract 185→184; linha `atom` no
`close_proofs.tsv` (entry `next_txn_id_after`); catálogo
`data_fate` removido com `atom_reason` datado; residuals
atualizados.

## Escada

cap_data_fate 31→30, floor_atom 93→94, floor_extract 185→184.

## Gates

`check_depth_floor.py` GREEN: extract=184 (floor 184), atom=94
(floor 94), data_fate=30<=30, residuals == live.
