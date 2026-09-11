# RFC-0212 P1.2 — átomo `catalog:drop_sent_through` (cadência membership 6/6, 6/6 FECHAMENTO)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit); fecha o P1.2 e o
bloco membership inteiro

## O que foi pago

```lean
theorem drop_sent_through_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (drop_sent_through in_ids = ok v) ↔
        ((v = true ∧ in_ids = false)
          ∨ (v = false ∧ in_ids = true))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0148: o bookkeeping `sent_through` de um nó removido de `ids` por
um remove out-of-band é esquecido — exatamente quando o nó está fora
de `ids`. Corpo invertido (`!in_ids`): o destino `true` é o DROP.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn drop_sent_through(in_ids: bool) -> bool {
    !in_ids
}
```

## AS-IS recusado (a mentira)

```rust
pub fn drop_sent_through_as_is(_in_ids: bool) -> bool {
    false
}
```

A sobra 0147: mantém o `sent_through` depois do oob remove_member — o
mapa continua achando que já mandou tudo para um nó que não existe mais
na membresia. O dente DST no kernel e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- drop_sent_through_on_live_queued_is_not_ok
test three_teeth_queued::drop_sent_through_on_live_queued_is_not_ok ... ok
```

(As 6 plantas do P1.2 rodadas juntas em paralelo: 6 passed, 1.51s.)

## Cirurgia de catálogo

`promote_atom.py drop_sent_through drop_sent_through_fate_iff ...`:
floor_atom 90→91, floor_extract 188→187; linha `atom` no
`close_proofs.tsv` (entry `drop_sent_through`); catálogo `data_fate`
removido com `atom_reason` datado; residuals atualizados.

## Fechamento P1.2 — números exatos

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 39 | 33 |
| floor_atom | 85 | 91 |
| floor_extract | 193 | 187 |

6 atoms, 6 commits: `fe29d5d4` (pending_joint_node), `142b2efb`
(joint_target), `1223b824` (joint_add_target), `38b468b4`
(joint_leave_ok), `a8776aa8` (drop_repl_slot), este (drop_sent_through).

## Membership ZERO data_fate (medido ao vivo)

Consulta pós-cirurgia no catálogo ao vivo:

```
membership data_fate restantes: ZERO
total data_fate restantes (catalogo): 33
```

O kernel `membership_kernel.rs` teve seus 22 pares TODOS promovidos a
`atom`/`close` com teorema iff sobre corpo real + planta DST verde
(P0.1 ×4 + P0.2 ×4 + P1.1 ×8 + P1.2 ×6 = 22/22).

## Gates (fechamento, 3× GREEN)

- `check_depth_floor.py`: GREEN — extract=187 (floor 187), atom=91
  (floor 91), data_fate=33<=33, residuals == live
- `check_product_floor.py`: GREEN — promoted=4>=floor 4
- `check_ledger_consistency.py`: GREEN — 19 ponteiros resolvem,
  total=292 proof=266 campaign=26
