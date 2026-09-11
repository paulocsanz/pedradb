# RFC-0212 P1.1 — átomo `catalog:recover_apply_node` (cadência membership 3/6, 1/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

Teorema iff de destino sobre o corpo real extraído que o rustc liga:

```lean
theorem recover_apply_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (recover_apply_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0131: o recover aplica em TODA réplica local — exatamente quando o
nó é local. Ser membro de `ids` não é o portão; uma réplica removida
continua aplicando seu recovery.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn recover_apply_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}
```

## AS-IS recusado (a mentira)

```rust
pub fn recover_apply_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}
```

A sobra 0130: a réplica removida de `ids` é pulada no recover — estado
aplicado divergente em nó que já votou. O dente DST no kernel e a
planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- recover_apply_node_counts_on_live_queued_is_not_ok
test three_teeth_queued::recover_apply_node_counts_on_live_queued_is_not_ok ... ok
```

(8 plantas do P1.1 rodadas juntas em paralelo: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py recover_apply_node recover_apply_node_counts_fate_iff ...`:

- `proof_depth.tsv`: floor_atom 77→78, floor_extract 201→200
- `close_proofs.tsv`: linha `atom` TAB (entry `recover_apply_node_counts`)
- catálogo: `data_fate` removido, `atom_reason` datado 2026-09-11
- residuals: `glue.proof_depth.atom/extract`, `glue.data_fate`

## Escada

cap_data_fate 47→46, floor_atom 77→78, floor_extract 201→200.

## Gates

`check_depth_floor.py` GREEN: extract=200 (floor 200), atom=78 (floor 78),
data_fate=46<=46, residuals == live.
