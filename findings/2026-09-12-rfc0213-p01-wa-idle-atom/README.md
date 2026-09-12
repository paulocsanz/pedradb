# RFC-0213 P0.1 — átomo `catalog:write_admission` (cadência storage 1/9)

**Data:** 2026-09-12
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem write_admission_idle_fate_iff :
    ∀ (mem_stall pressure_l0 stall_l0 v : Bool),
      (write_admission_idle mem_stall pressure_l0 stall_l0 = ok v) ↔
        ((v = true ∧ ¬mem_stall ∧ ¬pressure_l0 ∧ ¬stall_l0)
          ∨ (v = false ∧ (mem_stall ∨ pressure_l0 ∨ stall_l0)))
```

`formal/aeneas/lean/WriteAdmission.lean` (build `lake build
WriteAdmission` verde).

## Semântica

RFC-0170 P2.4: o caminho idle de write-admission está livre
EXATAMENTE quando nenhum knob de stall está armado — mem-stall,
pressure-l0 e stall-l0 todos desligados.

Corpo (`crates/pedradb-core/src/write_admission_kernel.rs`):

```rust
pub fn write_admission_idle(mem_stall: bool, pressure_l0: bool, stall_l0: bool) -> bool {
    idle_body!(mem_stall, pressure_l0, stall_l0)  // !mem && !pressure && !stall
}
```

## AS-IS recusado (a mentira)

```rust
pub fn write_admission_idle_as_is(...) -> bool { true }
```

Sempre idle: knobs de stall decorativos — escrita admitida com
mem/L0 estourados. A planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-core && cargo test --lib -- write_admission_idle_on_live_stall_is_not_ok
test write_admission_kernel::tests::write_admission_idle_on_live_stall_is_not_ok ... ok
```

(No lote paralelo das 9 do P0.1: 9 passed, 0 failed.)

## Cirurgia de catálogo

`promote_atom.py write_admission write_admission_idle_fate_iff ...`:
floor_atom 98→99, floor_extract 180→179; linha `atom` no
`close_proofs.tsv` (entry `write_admission_idle`); catálogo
`data_fate` removido com `atom_reason` datado; residuals
atualizados.

## Escada

cap_data_fate 26→25, floor_atom 98→99, floor_extract 180→179.

## Gates

`check_depth_floor.py` GREEN: extract=179 (floor 179), atom=99
(floor 99), data_fate=25<=25, residuals == live.
