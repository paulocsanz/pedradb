# RFC-0191 P2.3 passo 27 — `encode_cf_key` graduado a atom (Fire 785)

**Data:** 2026-09-10 · **Par:** `encode_cf_key` · **Kernel:** `crates/pedradb-core/src/cf_kernel.rs` · **Finding:** RFC-0150 P0

## O que fechou

`encode_cf_key` — codificar (cf, key) para o byte-stream com prefixo —
agora tem iff forall sobre o corpo Aeneas extraído (`CfKernel.lean`):

```
encode_cf_key_ok_iff_bare_key_or_prefixed_vec
  ∀ (cf : Str) (key : Slice Std.U8) (default_raw : Bool) (v : Vec Std.U8),
  (encode_cf_key cf key default_raw = ok v) ↔
    ((∃ eff b, cf_encode_effective … = ok eff ∧ is_empty eff = ok b
      ∧ b = true ∧ Slice.to_vec key = ok v)
     ∨ (∃ eff b i i1 i3 s enc1 enc2, … ∧ ¬(b = true)
        ∧ len eff = ok i ∧ i + 1#usize = ok i1
        ∧ i1 + Slice.len key = ok i3 ∧ as_bytes eff = ok s
        ∧ extend_from_slice (with_capacity i3) s = ok enc1
        ∧ push enc1 0#u8 = ok enc2
        ∧ extend_from_slice enc2 key = ok v))
```

A semântica honesta: encoding efetivo vazio ⇒ a chave vai **nua**;
senão o output é a **cadeia planejada por capacidade** — bytes do cf,
um `0` separador, a chave — e o iff declara que o valor sai exatamente
quando cada passo monádico da cadeia é `ok` (incluindo o cálculo de
capacidade `len+1+key.len`). O AS-IS (prefixo dropado — colisão de
chaves entre CFs) é o dente já registrado.

## Prova

Molde dos passos 24–26: 7 inversões `bind_ok_inv` + 1 split.
LORE: os pure lets (`i2 := Slice.len key`, `out := with_capacity …`)
**já saem zeta-reduzidos** pela unificação do `obtain` — chamar
`simp only []`/`dsimp only []` ali vira erro de "no progress" (o note
de instances-transparency é ruído do reporter). Não chamar nada.

## Números

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 104 | **103** |
| floor_atom | 27 | **28** |
| floor_extract | 252 | **251** |

## Verificação (mesma árvore, antes do commit)

- `lake build Cf` — verde, **0 sorry**.
- `scripts/check_depth_floor.py` — GREEN (extract=251, atom=28,
  data_fate=103).
- `scripts/check_product_floor.py` — GREEN.
- Planta: `cargo test -p pedradb-sim --lib encode_cf_key`
  (`encode_cf_key_on_live_sst_bounds_is_not_ok`).
- `pedra_formal.py`: `ok encode_cf_key: proof_depth=atom (2026-09-10)`.

## Trampolim

`encode` (`rocksdb-compat/src/lib.rs`) e `compact_ssts_only_cf`
(`db.rs`) chamam o kernel Rust — mesma decisão provada.

## Próximo (786)

`infer_sst_cf` — o último da fila cf: match dos bounds (none/some ×
none/some), duas `cf_family_of` opacas e o eq de String no caso
(some, some). Depois: fila vazia, cap 102, déficit 2 — confrontar os
26 pares sem `aeneas`/10 sem entry ou re-scope honesto no RFC.
