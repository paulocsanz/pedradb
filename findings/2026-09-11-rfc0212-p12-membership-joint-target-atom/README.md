# RFC-0212 P1.2 — átomo `catalog:joint_target` (cadência membership 5/6, 2/6)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem joint_target_counts_fate_iff :
    ∀ (in_ids in_nodes : Bool) (v : Bool),
      (joint_target_counts in_ids in_nodes = ok v) ↔
        ((v = true ∧ in_ids = true)
          ∨ (v = false ∧ in_ids = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0119: o alvo de um joint-remove é o conjunto de membresia (`ids`) —
exatamente quando o alvo está em `ids`. O mapa local `nodes` não é o
portão: uma réplica TCP só tem a si mesma lá.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn joint_target_counts(in_ids: bool, _in_nodes: bool) -> bool {
    in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn joint_target_counts_as_is(in_ids: bool, in_nodes: bool) -> bool {
    in_nodes
}
```

A sobra 0118: exige o alvo no mapa local `nodes` — uma réplica TCP não
consegue joint-remover um par (o alvo nunca está no mapa dela). O dente
DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- joint_target_counts_on_live_queued_is_not_ok
test three_teeth_queued::joint_target_counts_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 6 do P1.2: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py joint_target joint_target_counts_fate_iff ...`:
floor_atom 86→87, floor_extract 192→191; linha `atom` no
`close_proofs.tsv` (entry `joint_target_counts`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 38→37, floor_atom 86→87, floor_extract 192→191.

## Gates

`check_depth_floor.py` GREEN: extract=191 (floor 191), atom=87 (floor 87),
data_fate=37<=37, residuals == live.
