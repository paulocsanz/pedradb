# RFC-0212 P2.1 — átomo `catalog:unreserve_si_gen` (cadência txn 6/7)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem unreserve_si_gen_fate_iff :
    ∀ (current stamped v : U64),
      (unreserve_si_gen current stamped = ok v) ↔
        ((v = core.num.U64.saturating_sub stamped 1#u64
            ∧ stamped > 0#u64 ∧ current = stamped)
          ∨ (v = current
            ∧ (¬ (stamped > 0#u64) ∨ ¬ (current = stamped))))
```

`formal/aeneas/lean/StoreTxn.lean` (build `lake build StoreTxn` verde).

## Semântica

F49: desfaz uma reserva EXATAMENTE quando nada mais reservou
depois — `stamped > 0` e `current == stamped` → o contador volta a
`stamped−1`; qualquer outra combinação (gen já avançada, nada
carimbado) deixa o contador onde está.

Corpo (`crates/pedradb-store/src/txn_kernel.rs`):

```rust
pub fn unreserve_si_gen(current: u64, stamped: u64) -> u64 {
    if stamped > 0 && current == stamped {
        stamped.saturating_sub(1)
    } else {
        current
    }
}
```

## AS-IS recusado (a mentira)

```rust
pub fn unreserve_si_gen_as_is(_current: u64, stamped: u64) -> u64 {
    stamped.saturating_sub(1)
}
```

Rola o contador para trás mesmo quando outra reserva já avançou
(na proposta que falhou, o propose seguinte reemite a MESMA gen).
A planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- unreserve_si_gen_on_live_queued_is_not_ok
test three_teeth_queued::unreserve_si_gen_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

Técnica: dois `broadcast_append` de Put no cluster vivo geram gens
carimbadas distintas `(stale, current)`; `unreserve(current,
stale)` deve devolver `current` (gen avançada não rola para trás);
o as-is devolveria `stale`.

## Cirurgia de catálogo

`promote_atom.py unreserve_si_gen unreserve_si_gen_fate_iff ...`:
floor_atom 96→97, floor_extract 182→181; linha `atom` no
`close_proofs.tsv` (entry `unreserve_si_gen`); catálogo
`data_fate` removido com `atom_reason` datado; residuals
atualizados.

## Escada

cap_data_fate 28→27, floor_atom 96→97, floor_extract 182→181.

## Gates

`check_depth_floor.py` GREEN: extract=181 (floor 181), atom=97
(floor 97), data_fate=27<=27, residuals == live.
