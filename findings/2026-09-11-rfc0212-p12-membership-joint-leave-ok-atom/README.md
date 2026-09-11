# RFC-0212 P1.2 — átomo `catalog:joint_leave_ok` (cadência membership 5/6, 4/6)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem joint_leave_ok_fate_iff :
    ∀ (leave_in_log : Bool) (v : Bool),
      (joint_leave_ok leave_in_log = ok v) ↔
        ((v = true ∧ leave_in_log = true)
          ∨ (v = false ∧ leave_in_log = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0096: um joint commitado não é configuração única até que um leave
(`old == new`) esteja no log — o fim do joint conta exatamente quando a
entrada de leave está no log.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn joint_leave_ok(leave_in_log: bool) -> bool {
    leave_in_log
}
```

## AS-IS recusado (a mentira)

```rust
pub fn joint_leave_ok_as_is(_leave_in_log: bool) -> bool {
    true
}
```

A sobra 0066 (no caminho vivo de add): pula o leave-joint — o cluster
declara config única com o joint ainda aberto no log; duas maiores
"atuais" diferentes coexistem. O dente DST no kernel e a planta abaixo
refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- joint_leave_ok_on_live_queued_is_not_ok
test three_teeth_queued::joint_leave_ok_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 6 do P1.2: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py joint_leave_ok joint_leave_ok_fate_iff ...`:
floor_atom 88→89, floor_extract 190→189; linha `atom` no
`close_proofs.tsv` (entry `joint_leave_ok`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 36→35, floor_atom 88→89, floor_extract 190→189.

## Gates

`check_depth_floor.py` GREEN: extract=189 (floor 189), atom=89 (floor 89),
data_fate=35<=35, residuals == live.
