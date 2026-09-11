# RFC-0191 P2.3 passo 25 — `compact_rewrites_sst_cf` graduado a atom (Fire 783)

**Data:** 2026-09-10 · **Par:** `compact_rewrites_sst_cf` · **Kernel:** `crates/pedradb-core/src/cf_kernel.rs` · **Finding:** RFC-0150 P0

## O que fechou

`compact_rewrites_sst_cf` — se um compact de `family` reescreve um SST
tagueado `sst_cf` — agora tem iff forall sobre o corpo Aeneas extraído
(`CfKernel.lean`, `aeneas_cf.sh`):

```
compact_rewrites_sst_cf_ok_iff_empty_tag_never_or_representative_in_family
  ∀ (sst_cf : Str) (family : Str) (v : Bool),
  (compact_rewrites_sst_cf sst_cf family = ok v) ↔
    ((∃ b, is_empty sst_cf = ok b ∧ b = true ∧ v = false)
     ∨ (∃ b s enc, is_empty sst_cf = ok b ∧ ¬(b = true)
        ∧ lift (Array.to_slice (Std.Array.empty Std.U8)) = ok s
        ∧ encode_cf_key sst_cf s false = ok enc
        ∧ key_in_cf_family (Vec.deref enc) family = ok v))
```

A semântica honesta: **tag vazia (mista/legacy) nunca é reescrita** (valor
false); tag não-vazia é decidida pelo teste in-family sobre a **chave
representativa codificada** da tag — e o iff declara que o valor sai
exatamente quando cada passo monádico da rota (is_empty, lift do slice
vazio, encode do representante, key_in_cf_family) é `ok`. O AS-IS
(rewrite sempre) é o dente de vazamento de lock-compact já registrado.

## Prova

Molde do passo 24: lemas privados `bind_ok_inv`/`bind_intro` em Cf.lean
(`cases` em variável livre; nunca `rw … at h` sobre opaco). Inversão do
primeiro bind, `split at hval` no `if b`, mais duas inversões no ramo
não-vazio. **Verde na PRIMEIRA build.**

`lift` é `Aeneas.Std.lift x = ok x` por definição (Primitives.lean:269) —
mantido opaco na cadeia ∃, sem custo.

## Números

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 106 | **105** |
| floor_atom | 25 | **26** |
| floor_extract | 253 | **253→253** (254→253) |

## Verificação (mesma árvore, antes do commit)

- `lake build Cf` — verde, **0 sorry** no módulo, primeira tentativa.
- `scripts/check_depth_floor.py` — GREEN (extract=253, atom=26,
  data_fate=105).
- `scripts/check_product_floor.py` — GREEN (promoted 4/4).
- Planta: `cargo test -p pedradb-sim --lib compact_rewrites_sst_cf`
  (`compact_rewrites_sst_cf_on_live_meta_is_not_ok`).
- `pedra_formal.py`: `ok compact_rewrites_sst_cf: proof_depth=atom
  (2026-09-10)`; 146 FAILs herdados, nenhum nomeia o fire.

## Trampolim

`compact_ssts_only_cf` (`db.rs`) chama o kernel Rust — a decisão do
trampolim casa com a do kernel provado.

## Próximo (784)

`decode_cf_key` — mesmo molde (effective → is_empty → if → len/+1/get
range); depois `encode_cf_key` (cadeia de extends), `infer_sst_cf`
(match de Options + duas cf_family_of). Fila 3, cap 105, faltam 5
descidas p/ ≤100 — déficit 2 segue em aberto.
