# RFC-0212 P0.2 — átomo `catalog:hint_member` (cadência membership 2/6, 4/4 FECHAMENTO)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit); fecha o P0.2

## O que foi pago

Teorema iff de destino sobre o corpo real extraído que o rustc liga:

```lean
theorem hint_if_member_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (hint_if_member in_ids = ok v) ↔
        ((v = true ∧ in_ids = true)
          ∨ (v = false ∧ in_ids = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0146: o hint de roteamento do líder conta exatamente quando o nó
apontado está em `ids`. Um `leader_id` fora da membresia não é hint —
é um ponteiro para um nó que não decide mais nada.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn hint_if_member(in_ids: bool) -> bool {
    in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn hint_if_member_as_is(_in_ids: bool) -> bool {
    true
}
```

A sobra 0145: qualquer `leader_id` é devolvido como hint — inclusive o
de um nó removido. O dente DST no kernel
(`hint_if_member_as_is` assert) e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- hint_if_member_on_live_queued_is_not_ok
test three_teeth_queued::hint_if_member_on_live_queued_is_not_ok ... ok
1 passed; 0 failed; 545 filtered out; 0.23s
```

(`three_teeth_queued` é módulo da lib, não target de integração — roda
via `--lib`.)

## Cirurgia de catálogo

`promote_atom.py hint_member hint_if_member_fate_iff ...`:

- `proof_depth.tsv`: floor_atom 76→77, floor_extract 202→201
- `close_proofs.tsv`: linha `atom` TAB
  `hint_if_member_fate_iff` (entry `hint_if_member`)
- catálogo: `data_fate` removido, `atom_reason` datado 2026-09-11
- residuals: `glue.proof_depth.atom/extract`, `glue.data_fate`

## Fechamento P0.2 — números exatos

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 51 | 47 |
| floor_atom | 73 | 77 |
| floor_extract | 205 | 201 |

4 atoms, 4 commits: `2b953b9a` (persist_meta), `c725658a`
(persist_hist), `bc6b6f80` (persist_fence), este (hint_member).

## Gates (fechamento, 3× GREEN)

- `check_depth_floor.py`: GREEN — extract=201 (floor 201), atom=77
  (floor 77), data_fate=47<=47, residuals == live
- `check_product_floor.py`: GREEN — promoted=4>=floor 4
- `check_ledger_consistency.py`: GREEN — 19 ponteiros resolvem,
  total=292 proof=266 campaign=26
