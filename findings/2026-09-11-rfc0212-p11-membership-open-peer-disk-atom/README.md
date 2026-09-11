# RFC-0212 P1.1 — átomo `catalog:open_peer_disk` (cadência membership 4/6, 5/8)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

```lean
theorem open_peer_uses_disk_fate_iff :
    ∀ (has_disk : Bool) (v : Bool),
      (open_peer_uses_disk has_disk = ok v) ↔
        ((v = true ∧ has_disk = true)
          ∨ (v = false ∧ has_disk = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0140: o open in-process carrega os peers do raft do disco de
membership — exatamente quando há membership em disco. O CLI `n_nodes`
não define a membresia de um diretório reaberto.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn open_peer_uses_disk(has_disk: bool) -> bool {
    has_disk
}
```

## AS-IS recusado (a mentira)

```rust
pub fn open_peer_uses_disk_as_is(_has_disk: bool) -> bool {
    false
}
```

A sobra 0125: peers vindos do CLI `1..=n_nodes` — só o TCP espiava o
disco; um reopen com membresia encolhida fala com nós fantasmas. O
dente DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- open_peer_uses_disk_on_live_queued_is_not_ok
test three_teeth_queued::open_peer_uses_disk_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 8 do P1.1: 8 passed, 1.93s.)

## Cirurgia de catálogo

`promote_atom.py open_peer_disk open_peer_uses_disk_fate_iff ...`:
floor_atom 81→82, floor_extract 197→196; linha `atom` no
`close_proofs.tsv` (entry `open_peer_uses_disk`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Escada

cap_data_fate 43→42, floor_atom 81→82, floor_extract 197→196.

## Gates

`check_depth_floor.py` GREEN: extract=196 (floor 196), atom=82 (floor 82),
data_fate=42<=42, residuals == live.
