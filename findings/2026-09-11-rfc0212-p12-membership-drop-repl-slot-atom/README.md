# RFC-0212 P1.2 — átomo `catalog:drop_repl_slot` (cadência membership 6/6, 5/6)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem drop_repl_slot_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (drop_repl_slot in_ids = ok v) ↔
        ((v = true ∧ in_ids = false)
          ∨ (v = false ∧ in_ids = true))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0147: o slot de replicação (`next`/`match`) de um nó removido de
`ids` é esquecido — exatamente quando o nó está fora de `ids`. Corpo
invertido (`!in_ids`): o destino `true` é o DROP.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn drop_repl_slot(in_ids: bool) -> bool {
    !in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn drop_repl_slot_as_is(_in_ids: bool) -> bool {
    false
}
```

A sobra 0146: mantém os slots de replicação depois do joint leave — o
líder continua rastreando `next`/`match` de um nó que já não é membro,
reenviando para um fantasma. O dente DST no kernel e a planta abaixo
refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- drop_repl_slot_on_live_queued_is_not_ok
test three_teeth_queued::drop_repl_slot_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 6 do P1.2: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py drop_repl_slot drop_repl_slot_fate_iff ...`:
floor_atom 89→90, floor_extract 189→188; linha `atom` no
`close_proofs.tsv` (entry `drop_repl_slot`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 35→34, floor_atom 89→90, floor_extract 189→188.

## Gates

`check_depth_floor.py` GREEN: extract=188 (floor 188), atom=90 (floor 90),
data_fate=34<=34, residuals == live.
