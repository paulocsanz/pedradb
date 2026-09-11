# RFC-0191 P2.3 passo 26 — `decode_cf_key` graduado a atom (Fire 784)

**Data:** 2026-09-10 · **Par:** `decode_cf_key` · **Kernel:** `crates/pedradb-core/src/cf_kernel.rs` · **Finding:** RFC-0150 P0

## O que fechou

`decode_cf_key` — decodificar uma chave codificada de volta ao user key —
agora tem iff forall sobre o corpo Aeneas extraído (`CfKernel.lean`):

```
decode_cf_key_ok_iff_identity_or_stripped_past_prefix
  ∀ (cf : Str) (encoded : Slice Std.U8) (default_raw : Bool) (v : Slice Std.U8),
  (decode_cf_key cf encoded default_raw = ok v) ↔
    ((∃ eff b, cf_encode_effective … = ok eff ∧ is_empty eff = ok b
      ∧ b = true ∧ encoded = v)
     ∨ (∃ eff b i i1, … ∧ ¬(b = true) ∧ len eff = ok i ∧ i + 1#usize = ok i1
        ∧ ((i1 > Slice.len encoded ∧ lift (empty) = ok v)
           ∨ (¬(i1 > Slice.len encoded)
              ∧ Slice.index encoded { start := i1 } = ok v))))
```

A semântica honesta em três dentes: (1) encoding efetivo vazio ⇒ o
decode é a **identidade** (o buffer volta inteiro); (2) prefixo
`cf\0` cabe ⇒ a fatia a partir de `len+1`; (3) prefixo NÃO cabe ⇒ o
slice **vazio** — nunca uma vista parcial, deslocada ou sobrando. O
AS-IS (prefixo vaza no user key) é o dente já registrado.

## Prova

Molde dos passos 24/25 (helpers `bind_ok_inv`/`bind_intro`). Duas
inversões de bind + `split` no `is_empty`; mais duas inversões + `split`
no `i1 > n`. O `let n := Slice.len encoded` puro some com
`simp only []` (zeta) antes do segundo `split`.

LORE: em padrões anônimos `⟨…⟩`, os **binders ∃ vêm antes** dos
componentes ∧ — `∃ eff b, A ∧ B` é `⟨eff, b, A, B⟩`, não `⟨eff, A, b, B⟩`.

## Números

| métrica | antes | depois |
|---|---|---|
| cap_data_fate | 105 | **104** |
| floor_atom | 26 | **27** |
| floor_extract | 253 | **252** |

## Verificação (mesma árvore, antes do commit)

- `lake build Cf` — verde, **0 sorry**.
- `scripts/check_depth_floor.py` — GREEN (extract=252, atom=27,
  data_fate=104).
- `scripts/check_product_floor.py` — GREEN.
- Planta: `cargo test -p pedradb-sim --lib decode_cf_key`
  (`decode_cf_key_on_live_sst_bounds_is_not_ok`).
- `pedra_formal.py`: `ok decode_cf_key: proof_depth=atom (2026-09-10)`.

## Trampolim

`decode` (`rocksdb-compat/src/lib.rs`) chama o kernel Rust — o decode do
drop-in compat usa a mesma decisão provada.

## Próximo (785)

`encode_cf_key` — cadeia de extends (len → +1 → +key.len →
with_capacity → as_bytes → extend → push 0 → extend), ∃ por passo.
Depois `infer_sst_cf` (match de Options + duas `cf_family_of` + eq).
Fila 2, cap 104, faltam 4 descidas p/ ≤100 — déficit 2 em aberto.
