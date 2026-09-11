# RFC-0212 P1.2 — átomo `catalog:pending_joint_node` (cadência membership 5/6, 1/6)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem pending_joint_node_counts_fate_iff :
    ∀ (is_member : Bool) (v : Bool),
      (pending_joint_node_counts is_member = ok v) ↔
        ((v = true ∧ is_member = true)
          ∨ (v = false ∧ is_member = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0105: o joint pendente é definido só pelos logs dos membros atuais —
o log de um nó removido não define configuração nenhuma.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn pending_joint_node_counts(is_member: bool) -> bool {
    is_member
}
```

## AS-IS recusado (a mentira)

```rust
pub fn pending_joint_node_counts_as_is(_is_member: bool) -> bool {
    true
}
```

A sobra 0104: escaneia todo nó aberto, inclusive removido — o log morto
de um nó fora da membresia continua "definindo" o joint pendente. O
dente DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- pending_joint_node_counts_on_live_queued_is_not_ok
test three_teeth_queued::pending_joint_node_counts_on_live_queued_is_not_ok ... ok
```

(6 plantas do P1.2 rodadas juntas em paralelo: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py pending_joint_node pending_joint_node_counts_fate_iff ...`:
floor_atom 85→86, floor_extract 193→192; linha `atom` no
`close_proofs.tsv` (entry `pending_joint_node_counts`); catálogo
`data_fate` removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 39→38, floor_atom 85→86, floor_extract 193→192.

## Gates

`check_depth_floor.py` GREEN: extract=192 (floor 192), atom=86 (floor 86),
data_fate=38<=38, residuals == live.
