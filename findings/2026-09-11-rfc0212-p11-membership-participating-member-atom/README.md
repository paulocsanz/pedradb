# RFC-0212 P1.1 — átomo `catalog:participating_member` (cadência membership 4/6, 8/8 FECHAMENTO)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit); fecha o P1.1

## O que foi pago

```lean
theorem participating_if_member_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (participating_if_member in_ids = ok v) ↔
        ((v = true ∧ in_ids = true)
          ∨ (v = false ∧ in_ids = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0127: um nó participa se e somente se está no voter set atual.
Participação não é flag capturada — é consequência da membresia viva.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn participating_if_member(in_ids: bool) -> bool {
    in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn participating_if_member_as_is(_in_ids: bool) -> bool {
    true
}
```

A sobra 0126: mantém a flag `participating` capturada — o nó removido
continua contando como participante. O dente DST no kernel e a planta
abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- participating_if_member_on_live_queued_is_not_ok
test three_teeth_queued::participating_if_member_on_live_queued_is_not_ok ... ok
```

(As 8 plantas do P1.1 rodadas juntas em paralelo: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py participating_member participating_if_member_fate_iff ...`:
floor_atom 84→85, floor_extract 194→193; linha `atom` no
`close_proofs.tsv` (entry `participating_if_member`); catálogo
`data_fate` removido com `atom_reason` datado; residuals atualizados.

## Fechamento P1.1 — números exatos

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 47 | 39 |
| floor_atom | 77 | 85 |
| floor_extract | 201 | 193 |

8 atoms, 8 commits: `e93c7b0f` (recover_apply_node), `909dfb62`
(recover_truncate), `4b3c6ee8` (recover_abort), `f2497120`
(identity_before_applied), `96001f96` (open_peer_disk), `e3f59f57`
(local_id_member), `32b85288` (reader_local), este (participating_member).

## Gates (fechamento, 3× GREEN)

- `check_depth_floor.py`: GREEN — extract=193 (floor 193), atom=85
  (floor 85), data_fate=39<=39, residuals == live
- `check_product_floor.py`: GREEN — promoted=4>=floor 4
- `check_ledger_consistency.py`: GREEN — 19 ponteiros resolvem,
  total=292 proof=266 campaign=26
