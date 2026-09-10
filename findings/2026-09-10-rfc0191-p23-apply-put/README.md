# RFC-0191 P2.3 (passo 2) — apply-path Put fate sai do trampolim (cap 129→128)

**Data:** 2026-09-10 · **Fire:** 760 · **Estado:** landed

## Alvo

O ramo `Put` de `apply_range` (`crates/pedradb-store/src/lib.rs`) decidia
inline para onde cada record aplicado ia: pular chave reservada, aplicar
valor, e persistir hist só com `si_gen > 0`. Três destinos de dados no
handler — segundo `if` da cadência P2.3.

## O que corre no rustc agora

```rust
// crates/pedradb-store/src/apply_kernel.rs
pub enum ApplyPutFate { Skip, ApplyOnly, ApplyAndHist }
pub fn apply_put_plan(is_reserved: bool, si_gen: u64) -> ApplyPutFate
```

`apply_range` faz `match apply_put_plan(is_reserved_store_key(key),
*si_gen)` — mesma ordem/semântica (Skip reservado primeiro; gen-0 nunca
persiste hist). O ramo `Batch` fica para um fire futuro (a cadência é um
`if` por commit).

## Teorema (atom registrado)

`apply_put_plan_hist_iff_live_and_gen_positive` (`StoreApply.lean`, ∀
`is_reserved si_gen`): plano = `ApplyAndHist` ↔ (`is_reserved = false` ∧
`si_gen ≠ 0#u64`). 0 sorry. AS-IS dente `apply_put_plan_as_is` sempre
`ApplyAndHist` (stompa chave reservada e o piso).

### Lean lore novo (Fire 760)

O `split` sobre o GOAL no Lean 4.31 é recursivo E usa hipóteses do
contexto para descarregar condições de `if` internos (o `hz : si_gen ≠
0#u64` fecha o ramo `si_gen = 0` sozinho) — os ramos são dinâmicos, não
2^n fixos. Provar backward com `rintro ⟨hf, hz⟩` + `split` + 2 bullets
(then-branch com `next c1`; else já reduzido → `rfl`). `split at h`
(hipótese) NÃO é recursivo — forward com `split at h` + `next` aninhado
funciona como no P1.5.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 129 | **128** (`apply_step` gradua) |
| `floor_atom` | 2 | **3** (linha `atom catalog:apply_put_plan`) |
| residuals `proof_depth` | 275/2/2/17 | **275/2/3/17** |
| residuals `data_fate`/`single_artifact` | 129/289 | **128/290** |

## Evidência

- `cargo test -p pedradb-store --lib apply` → verde (planta
  `apply_put_plan_on_live_reserved_and_floor` + twins apply existentes).
- `lake build StoreApply` → `✔ Built StoreApply`, 0 sorry.
- depth-floor GREEN (atom=3/floor 3, data_fate 128≤128); product-floor
  GREEN (promoted 4).
- `scripts/aeneas_store_apply.sh` regenerado (`StoreApplyKernel.lean` +
  `SOURCE.store_apply`).

## Churn paralelo (registro)

Durante o fire, a sessão otimizar quebrou transitoriamente
`pedradb-core`/`pedradb-sim` (`payload_bytes`, `scale_kernel`) — assentou
sozinho; nada deste commit toca nesses arquivos.
