# RFC-0212 P2.1 — átomo `catalog:reserve_si_gen` (cadência txn 5/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem reserve_si_gen_fate_iff :
    ∀ (current : U64) (r : SiGenReserve),
      (reserve_si_gen current = ok r) ↔
        (r.next_current = core.num.U64.saturating_add current 1#u64
          ∧ r.reserved = core.num.U64.saturating_add current 1#u64)
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F49: reservar uma SI generation AVANÇA o contador **e** devolve o
novo valor — duas reservas pendentes carregam gens distintas.

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn reserve_si_gen(current: u64) -> SiGenReserve {
    let n = current.saturating_add(1);
    SiGenReserve { next_current: n, reserved: n }
}
```

## AS-IS recusado (a mentira)

```rust
pub fn reserve_si_gen_as_is(current: u64) -> SiGenReserve {
    SiGenReserve {
        next_current: current,
        reserved: current.saturating_add(1),
    }
}
```

Devolve `current+1` mas deixa o contador parado: a próxima
reserva devolve a MESMA gen — colisão de geração pendente. A
planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- reserve_si_gen_on_live_queued_is_not_ok
test three_teeth_queued::reserve_si_gen_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

Técnica: dois `broadcast_append` de Put no cluster vivo — o log RAM
do líder mostra gens carimbadas consecutivas distintas (2, 3);
o as-is carimbaria iguais.

## Cirurgia de catálogo

`promote_atom.py reserve_si_gen reserve_si_gen_fate_iff ...`:
floor_atom 95→96, floor_extract 183→182; linha `atom` no
`close_proofs.tsv` (entry `reserve_si_gen`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 29→28, floor_atom 95→96, floor_extract 183→182.

## Gates

`check_depth_floor.py` GREEN: extract=182 (floor 182), atom=96
(floor 96), data_fate=28<=28, residuals == live.
