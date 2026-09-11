# RFC-0212 P1.1 — átomo `catalog:reader_local` (cadência membership 4/6, 7/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem reader_id_local_fate_iff :
    ∀ (is_local : Bool) (v : Bool),
      (reader_id_local is_local = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0142: um fallback `ids.first()` de LocalApplied tem que ser nó
local — exatamente quando o nó do fallback é local. A leitura local não
pode ser atendida por um id remoto.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn reader_id_local(is_local: bool) -> bool {
    is_local
}
```

## AS-IS recusado (a mentira)

```rust
pub fn reader_id_local_as_is(_is_local: bool) -> bool {
    true
}
```

A sobra 0141: devolve `ids.first()` mesmo quando não é local — o reader
local marca leituras com id de outro nó. O dente DST no kernel e a
planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- reader_id_local_on_live_queued_is_not_ok
test three_teeth_queued::reader_id_local_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 8 do P1.1: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py reader_local reader_id_local_fate_iff ...`:
floor_atom 83→84, floor_extract 195→194; linha `atom` no
`close_proofs.tsv` (entry `reader_id_local`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 41→40, floor_atom 83→84, floor_extract 195→194.

## Gates

`check_depth_floor.py` GREEN: extract=194 (floor 194), atom=84 (floor 84),
data_fate=40<=40, residuals == live.
