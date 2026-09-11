# RFC-0212 P1.1 — átomo `catalog:identity_before_applied` (cadência membership 3/6, 4/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem membership_identity_before_applied_fate_iff :
    ∀ (identity_first : Bool) (v : Bool),
      (membership_identity_before_applied identity_first = ok v) ↔
        ((v = true ∧ identity_first = true)
          ∨ (v = false ∧ identity_first = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0124 P1.1: a identidade C-new persiste antes de avançar `applied`
passado o joint — exatamente quando o persist de identidade vem
primeiro. A ordem É a garantia: identidade durável antes do avanço que
a depende.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn membership_identity_before_applied(identity_first: bool) -> bool {
    identity_first
}
```

## AS-IS recusado (a mentira)

```rust
pub fn membership_identity_before_applied_as_is(_identity_first: bool) -> bool {
    false
}
```

O AS-IS persiste `applied` primeiro — a janela de crash: `applied`
avançado no disco enquanto os voters ficam para trás; após o crash o
nó aplica com uma membresia que nunca foi a prometida. O dente DST no
kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- membership_identity_before_applied_on_live_queued_is_not_ok
test three_teeth_queued::membership_identity_before_applied_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 8 do P1.1: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py identity_before_applied membership_identity_before_applied_fate_iff ...`:
floor_atom 80→81, floor_extract 198→197; linha `atom` no
`close_proofs.tsv` (entry `membership_identity_before_applied`); catálogo
`data_fate` removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 44→43, floor_atom 80→81, floor_extract 198→197.

## Gates

`check_depth_floor.py` GREEN: extract=197 (floor 197), atom=81 (floor 81),
data_fate=43<=43, residuals == live.
