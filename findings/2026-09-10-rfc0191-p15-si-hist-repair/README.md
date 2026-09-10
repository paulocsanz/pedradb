# RFC-0191 P1.5 — repair SI-hist: o `if` de destino sai do trampolim (cap 130→129)

**Data:** 2026-09-10 · **Fire:** 759 · **Estado:** landed

## Alvo

`repair_si_hist_tip` (`crates/pedradb-store/src/lib.rs`, F52/F117) decidia
inline se reescrevia a ponta do hist de SI: `if tip_gen == 0 || tip_matches
{ return Ok(()) }`. Dois `if`s de destino de dados vivos no handler — o
primeiro `if` da cadência P2.3 (cap `data_fate` ≤ 100).

## O que corre no rustc agora

```rust
// crates/pedradb-store/src/txn_kernel.rs
pub enum SiHistRepair { Leave, Rewrite }
pub fn si_hist_repair_plan(tip_gen: u64, tip_matches: bool) -> SiHistRepair {
    if tip_gen == 0 || tip_matches { SiHistRepair::Leave } else { SiHistRepair::Rewrite }
}
```

`repair_si_hist_tip` faz `match si_hist_repair_plan(...) { Leave => return
Ok(()), Rewrite => {} }` — mesma ordem/semântica (guard `batch_is_empty` e o
rewrite `hist.last_mut()` intactos).

## Teorema (atom registrado)

`si_hist_repair_plan_leave_iff_floor_or_match` (`formal/aeneas/lean/Txn.lean`,
∀ `tip_gen tip_matches`): plano = `Leave` ↔ (`tip_gen = 0#u64` ∨
`tip_matches = true`). Prova por `split` sobre o `ite` extraído — a forma de
coerção do `tip_gen = 0#u64` elaborado não é adivinhada à mão. 0 sorry
(só o ruído upstream Slice/StringIter no replay). AS-IS dente
`si_hist_repair_plan_as_is` sempre `Rewrite` (stompa o piso).

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 130 | **129** (`should_repair_si_hist` gradua; par novo nasce provado) |
| `floor_atom` | 1 | **2** (linha `atom catalog:si_hist_repair` no `close_proofs.tsv`) |
| `floor_extract` | 276 | **275** (recount — ver abaixo) |
| residuals `proof_depth` | 276/2/1/17 | **275/2/2/17** |
| residuals `data_fate`/`single_artifact` | 130/288 | **129/289** |

### Recount do extract (achado deste slice)

A promoção do `visible_at` a atom (0085e919) tirou um par do extract sem
baixar o congelado: residuals dizia `extract: 276` com vivo 275 (o gate
`depth-floor` lê o freeze, não o vivo, nessa coluna; o `pedra_formal`
reclamava desde então). Corrigido aqui no mesmo commit:
`floor_extract` 276→275 + residuals 275 — promoção de escada, não extração
perdida.

### Chaves congeladas NÃO mexidas (herdadas)

`kernel_files` 59 / `kernel_loc` 23189 / `handler_loc` 112092 seguem
congelados: o vivo inclui churn não-commitado da sessão otimizar na árvore
compartilhada (`db.rs`, `concurrent.rs`, `write_cycle_kernel.rs` não
matriculado). Re-freeze quando esse WIP landar — mesma convenção dos lands
P2.1/P2.2.

## Evidência

- `cargo test -p pedradb-store --lib repair` → verde (planta
  `si_hist_repair_plan_on_live_gen0_floor_leaves` + revert 9).
- `lake build Txn` → `✔ Built Txn`.
- `python3 scripts/check_depth_floor.py` → GREEN (extract=275/floor 275,
  atom=2/floor 2, data_fate 129≤129).
- `python3 scripts/check_product_floor.py` → GREEN (promoted 4≥4).
- `scripts/aeneas_txn.sh` regenerado (`TxnKernel.lean` + `SOURCE.txn`).
