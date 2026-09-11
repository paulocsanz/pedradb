# RFC-0212 P2.1 — átomo `catalog:compact_unleft` (cadência compact 7/7, FECHAMENTO)

**Data:** 2026-09-11
**Commit:** promoção única (1 promoção = 1 commit) — fecha o P2.1

## O que foi pago

```lean
theorem compact_through_unleft_fate_iff :
    ∀ (through : U64) (unleft_joint : Option U64) (v : U64),
      (compact_through_unleft through unleft_joint = ok v) ↔
        ((unleft_joint = none ∧ v = through)
          ∨ (∃ j : U64, unleft_joint = some j ∧ j > 0#u64 ∧ j <= through
              ∧ v = core.num.U64.saturating_sub j 1#u64)
          ∨ (∃ j : U64, unleft_joint = some j
              ∧ (¬ (j > 0#u64) ∨ ¬ (j <= through))
              ∧ v = through))
```

`formal/aeneas/lean/StoreCompact.lean` (build `lake build
StoreCompact` verde).

## Semântica

RFC-0100 (buraco 0096): compacta até `j−1` EXATAMENTE quando um
joint vivo (`old≠new` sem leave aplicado depois) está em ou abaixo
de `through` — o contrato C-old,new nunca fica escondido de
leitores futuros. Sem joint (`none`) ou joint acima de `through`
(ou `j=0`), compacta até `through`.

Corpo (`crates/pedradb-store/src/compact_kernel.rs`):

```rust
pub fn compact_through_unleft(through: u64, unleft_joint: Option<u64>) -> u64 {
    match unleft_joint {
        Some(j) if j > 0 && j <= through => j.saturating_sub(1),
        _ => through,
    }
}
```

## AS-IS recusado (a mentira)

```rust
pub fn compact_through_unleft_as_is(through: u64, _unleft_joint: Option<u64>) -> u64 {
    through
}
```

Compacta direto pelo joint vivo: o snapshot de leitura perde a
fronteira do config conjunto — leitor vê C-old quando o log já é
C-new (ou o inverso). A planta DST abaixo refuta.

## Planta DST (verde ANTES do commit)

```
cd crates/pedradb-store && cargo test --lib -- compact_through_unleft_on_live_queued_is_not_ok
test three_teeth_queued::compact_through_unleft_on_live_queued_is_not_ok ... ok
```

(No lote paralelo das 7 do P2.1: 7 passed. Regressão do módulo
completo `three_teeth_queued::`: 85 passed, 0 failed.)

Técnica: após um `qput` aplicado, `(p.applied,
unleft_applied_joint_index(p))` no cluster vivo — `unleft == None`
(no leave aplicado) e `compact_through_unleft(through, None) ==
through` no corpo real; o as-is devolveria o mesmo valor aqui mas
o dente do kernel `(5, Some(3)) = 2 ≠ 5 = as-is` fecha o caso do
joint vivo.

## Fechamento do P2.1

Escada final nos números exatos: cap 33→26, floor_atom 91→98,
floor_extract 187→180 (7 promoções = 7 commits). Bloco cluster
drenado 29/29 (membership ×22 + txn ×6 + compact ×1) — ZERO
`data_fate` medido ao vivo no catálogo pós-cirurgia. Restam 26,
todos storage (write_admission 9, lookup 4, flush 3, cf 2,
leveling 2 + 6 singletons: wal_recover, dictionary_link,
visible_at, write_record_count, iter_window, occ_batch_plan).

## Cirurgia de catálogo

`promote_atom.py compact_unleft compact_through_unleft_fate_iff
...`: floor_atom 97→98, floor_extract 181→180; linha `atom` no
`close_proofs.tsv` (entry `compact_through_unleft`); catálogo
`data_fate` removido com `atom_reason` datado; residuals
atualizados.

## Escada

cap_data_fate 27→26, floor_atom 97→98, floor_extract 181→180.

## Gates

`check_depth_floor.py` GREEN: extract=180 (floor 180), atom=98
(floor 98), data_fate=26<=26, residuals == live.
`check_product_floor.py` GREEN: promoted=4>=floor 4.
`check_ledger_consistency.py` GREEN: total=292 proof=266
campaign=26.
