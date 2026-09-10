# RFC-0191 P2.3 (passo 3) — merge do SI-hist na carga sai do trampolim (cap 128→127)

**Data:** 2026-09-10 · **Fire:** 761 · **Estado:** landed

## Alvo

O loop de réplica de `load_si_from_disk` (`crates/pedradb-store/src/lib.rs`,
F36/F119) decidia inline, por record: hist decodificado substitui o
best-so-far? hist corrupto é ignorado ou marcado corrupt-only? O F119 é o
product guarantee por trás: **réplica corrupta nunca apaga estado SI**;
all-corrupt falha fechado no caller (`batch_is_empty(corrupt_only)`).

## O que corre no rustc agora

```rust
// crates/pedradb-store/src/txn_kernel.rs
pub enum HistLoadFate { MergeNew, KeepOld, TrackCorruptOnly }
pub fn hist_load_fate(decoded_ok: bool, best_has_user: bool,
                      new_last: u64, existing: u64) -> HistLoadFate
```

`MergeNew` iff decodificado ∧ `new_last >= existing`; corrupto com cópia
boa → `KeepOld`; corrupto sem cópia → `TrackCorruptOnly`. O trampolim
pergunta ao kernel nos DOIS ramos do `match decode_hist` (o lado Err passa
`(false, best_has, 0, 0)` — os zeros só alimentam o ramo inatingível).
Ordem/semântica preservadas (incl. `corrupt_only.remove` no ramo Ok).

## Teorema (atom registrado)

`hist_load_fate_merge_new_iff_decoded_and_not_below` (`Txn.lean`, ∀
`decoded_ok best_has_user new_last existing`): plano = `MergeNew` ↔
(`decoded_ok = true` ∧ `new_last >= existing`). 0 sorry. AS-IS dente
`hist_load_fate_as_is` sempre `MergeNew` (a réplica corrupta ganha — F119).

### Lean lore (Fire 761)

- `rw [if_pos h, if_pos h2]` FECHA o goal sozinho (o `rfl` extra vira
  erro "No goals"). Quando a condição do enunciado elabora igual à do
  extrato (mesma sintaxe de superfície), `if_pos` funciona — o
  `rw`-com-rfl embutido é o backward mais curto.
- `split at h` num else que contém `if` aninhado precisa de MAIS um
  `split at h` por nível (o `absurd h (by simp)` só fecha quando h já é
  construtor ≠ construtor).

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 128 | **127** (`recover_si_generation` gradua) |
| `floor_atom` | 3 | **4** (linha `atom catalog:hist_load_fate`) |
| residuals `proof_depth` | 275/2/3/17 | **275/2/4/17** |
| residuals `data_fate`/`single_artifact` | 128/290 | **127/291** |

## Evidência

- `cargo test -p pedradb-store --lib hist_load_fate` → verde (planta
  `hist_load_fate_on_live_merge_and_corrupt`, 5 asserts).
- `lake build Txn` → `✔ Built Txn`, 0 sorry.
- depth-floor GREEN (atom=4/floor 4, data_fate 127≤127); product-floor
  GREEN (promoted 4).
- `scripts/aeneas_txn.sh` regenerado (`TxnKernel.lean` + `SOURCE.txn`).
