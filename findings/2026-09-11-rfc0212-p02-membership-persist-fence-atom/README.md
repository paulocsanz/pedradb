# RFC-0212 P0.2 — átomo `catalog:persist_fence` (cadência membership 2/6, 3/4)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit)

## O que foi pago

Teorema iff de destino sobre o corpo real extraído que o rustc liga:

```lean
theorem persist_fence_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (persist_fence_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false))
```

`formal/aeneas/lean/Membership.lean` (build `lake build Membership` verde,
1699 jobs).

## Semântica

RFC-0137: a **cerca de aborto** (abort fence) persiste em TODA réplica
local — exatamente quando o nó é local. Ser membro de `ids` não é o
portão; uma réplica removida continua persistindo sua cerca.

Corpo (`crates/pedradb-raft/src/membership_kernel.rs`):

```rust
pub fn persist_fence_node_counts(is_local: bool, _in_ids: bool) -> bool {
    is_local
}
```

## AS-IS recusado (a mentira)

```rust
pub fn persist_fence_node_counts_as_is(is_local: bool, in_ids: bool) -> bool {
    is_local && in_ids
}
```

A sobra 0136: a réplica removida de `ids` fica **sem cerca** — janela de
aborto não-durável em nó que já aplicou. O dente DST no kernel
(`persist_fence_node_counts_as_is` assert) e a planta abaixo refutam.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- persist_fence_node_counts_on_live_queued_is_not_ok
test three_teeth_queued::persist_fence_node_counts_on_live_queued_is_not_ok ... ok
1 passed; 0 failed; 545 filtered out; 0.57s
```

(`three_teeth_queued` é módulo da lib, não target de integração — roda
via `--lib`.)

## Cirurgia de catálogo

`promote_atom.py persist_fence persist_fence_node_counts_fate_iff ...`:

- `proof_depth.tsv`: floor_atom 75→76, floor_extract 203→202
- `close_proofs.tsv`: linha `atom` TAB
  `persist_fence_node_counts_fate_iff` (entry `persist_fence_node_counts`)
- catálogo: `data_fate` removido, `atom_reason` datado 2026-09-11
- residuals: `glue.proof_depth.atom/extract`, `glue.data_fate`

## Escada (pós-commit esperado)

cap_data_fate 49→48, floor_atom 75→76, floor_extract 203→202.

## Gates

`check_depth_floor.py` GREEN: extract=202 (floor 202), atom=76 (floor 76),
data_fate=48<=48, residuals == live.
