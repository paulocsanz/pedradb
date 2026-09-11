# RFC-0212 P1.1 — átomo `catalog:recover_truncate` (cadência membership 3/6, 2/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem recover_truncate_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (recover_truncate_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0132: o log truncado no recover persiste em TODA réplica local —
exatamente quando o nó é local. `ids` não é o portão.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn recover_truncate_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}
```

## AS-IS recusado (a mentira)

```rust
pub fn recover_truncate_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}
```

A sobra 0131: a réplica removida mantém o sufixo — log não-truncado em
nó fora da membresia. O dente DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- recover_truncate_node_counts_on_live_queued_is_not_ok
test three_teeth_queued::recover_truncate_node_counts_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 8 do P1.1: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py recover_truncate recover_truncate_node_counts_fate_iff ...`:
floor_atom 78→79, floor_extract 200→199; linha `atom` no
`close_proofs.tsv` (entry `recover_truncate_node_counts`); catálogo
`data_fate` removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 46→45, floor_atom 78→79, floor_extract 200→199.

## Gates

`check_depth_floor.py` GREEN: extract=199 (floor 199), atom=79 (floor 79),
data_fate=45<=45, residuals == live.
