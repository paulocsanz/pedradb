# RFC-0191 P2.3 passo 28 — `infer_sst_cf` graduado a atom (Fire 786)

**Data:** 2026-09-10 · **Par:** `infer_sst_cf` · **Kernel:** `crates/pedradb-core/src/cf_kernel.rs` · **Finding:** RFC-0150 P0

## O que fechou

`infer_sst_cf` — a tag de CF de um SST a partir dos bounds — agora tem
iff forall sobre o corpo Aeneas extraído (`CfKernel.lean`), quatro
disposições:

```
infer_sst_cf_ok_iff_shared_family_or_empty
  ∀ (smallest largest : Option (Slice Std.U8)) (v : String),
  (infer_sst_cf smallest largest = ok v) ↔
    ((smallest = none ∧ largest = none ∧ String.new = ok v)
     ∨ (∃ s, smallest = none ∧ largest = some s ∧ cf_family_of s = ok v)
     ∨ (∃ s, smallest = some s ∧ largest = none ∧ cf_family_of s = ok v)
     ∨ (∃ s l a b b1, smallest = some s ∧ largest = some l
        ∧ cf_family_of s = ok a ∧ cf_family_of l = ok b
        ∧ eq a b = ok b1
        ∧ ((b1 = true ∧ a = v) ∨ (¬(b1 = true) ∧ String.new = ok v))))
```

A semântica honesta: (some,some) tagueia com a família **só quando os
dois bounds compartilham** uma (o eq decide; senão tag vazia); bound
único toma a família dele; sem bounds, tag vazia (mista/legacy).
**Nunca uma tag que minta sobre conteúdo misto** — o AS-IS (todo file
`default`) é o dente já registrado.

## Prova

Forward: `cases smallest <;> cases largest` — os três braços simples
fecham por **defeq puro** (match em ctor literal + `exact`); o
(some,some) é o molde ∃ (3 inversões + split no eq). Backward: rintro
único sobre a disjunção inteira + `subst` das equações de Option.

LORE: `subst` das equações de Option no backward substitui o match
scrutinee — os braços sem binds fecham por `exact h` (defeq iota).

## Números

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 103 | **102** |
| floor_atom | 28 | **29** |
| floor_extract | 251 | **250** |

## Verificação (mesma árvore, antes do commit)

- `lake build Cf` — verde, **0 sorry**, primeira tentativa.
- `scripts/check_depth_floor.py` — GREEN (extract=250, atom=29,
  data_fate=102).
- `scripts/check_product_floor.py` — GREEN.
- Planta: `cargo test -p pedradb-sim --lib infer_sst_cf`
  (`infer_sst_cf_on_live_flush_tag_is_not_ok`).
- `pedra_formal.py`: `ok infer_sst_cf: proof_depth=atom (2026-09-10)`.

## Trampolim

`write_l0_sst_for_family` (`sst/table.rs`) chama o kernel Rust.

## Estado da campanha

**Fila cf esgotada** — os 4 pares pesados (Str/Vec opacos) caíram no
mesmo molde ∃-cadeia, 2 na primeira build. Cap 102; alvo ≤100 exige 2
descidas além da fila. Próximos candidatos: os 26 pares sem campo
`aeneas` e 10 sem `entry` no catálogo (7 no cluster L28 =
cluster_real.rs — território P2.4/sessão paralela), ou re-scope
honesto no RFC-0191 se os pagamentos extra não couberem.
