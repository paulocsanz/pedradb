# RFC-0212 P1.1 — átomo `catalog:local_id_member` (cadência membership 4/6, 6/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem local_id_if_member_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (local_id_if_member in_ids = ok v) ↔
        ((v = true ∧ in_ids = true)
          ∨ (v = false ∧ in_ids = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0141: o nó local único é identidade deste processo exatamente
quando está em `ids` — a membresia atual define quem se é, não a
ordem de inserção.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn local_id_if_member(in_ids: bool) -> bool {
    in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn local_id_if_member_as_is(_in_ids: bool) -> bool {
    true
}
```

A sobra 0140: devolve a primeira chave do HashMap mesmo quando o nó foi
removido — o processo continua se achando membro que já não é. O dente
DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- local_id_if_member_on_live_queued_is_not_ok
test three_teeth_queued::local_id_if_member_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 8 do P1.1: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py local_id_member local_id_if_member_fate_iff ...`:
floor_atom 82→83, floor_extract 196→195; linha `atom` no
`close_proofs.tsv` (entry `local_id_if_member`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 42→41, floor_atom 82→83, floor_extract 196→195.

## Gates

`check_depth_floor.py` GREEN: extract=195 (floor 195), atom=83 (floor 83),
data_fate=41<=41, residuals == live.
