# RFC-0212 P1.2 — átomo `catalog:joint_add_target` (cadência membership 5/6, 3/6)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem joint_add_target_counts_fate_iff :
    ∀ (in_nodes : Bool) (v : Bool),
      (joint_add_target_counts in_nodes = ok v) ↔ v = true := by
  intro in_nodes v
  unfold joint_add_target_counts
  cases v <;> simp
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0119 P1.1: o alvo de um joint-add é sempre aceito — o processo que
está entrando é outro pid do SO que não precisa estar no mapa local
`nodes` (que só tem conexões já abertas). O corpo é constantemente
`true`; o iff diz exatamente isso: o destino decidido é sempre `true`.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn joint_add_target_counts(_in_nodes: bool) -> bool {
    true
}
```

## AS-IS recusado (a mentira)

```rust
pub fn joint_add_target_counts_as_is(in_nodes: bool) -> bool {
    in_nodes
}
```

A sobra 0119 P0: exige o joiner no mapa local `nodes` — ninguém consegue
entrar no cluster porque ainda não tem conexão aberta (o mapa só teria a
conexão DEPOIS de entrar). O dente DST no kernel e a planta abaixo
refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- joint_add_target_counts_on_live_queued_is_not_ok
test three_teeth_queued::joint_add_target_counts_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 6 do P1.2: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py joint_add_target joint_add_target_counts_fate_iff ...`:
floor_atom 87→88, floor_extract 191→190; linha `atom` no
`close_proofs.tsv` (entry `joint_add_target_counts`); catálogo
`data_fate` removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 37→36, floor_atom 87→88, floor_extract 191→190.

## Gates

`check_depth_floor.py` GREEN: extract=190 (floor 190), atom=88 (floor 88),
data_fate=36<=36, residuals == live.
