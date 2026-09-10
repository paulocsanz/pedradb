# RFC-0191 P2.3 (passo 6) — tx_glue graduado a atom (cap 125→124)

**Data:** 2026-09-10 · **Fire:** 764 · **Estado:** landed

## Alvo

O par `tx_glue` (F47/F34, `tx_glue_kernel.rs`, entry `tx_range_action`)
estava parado em `data_fate`. O kernel decide MajorityRevert /
LocalRevert / KeepCommitted desde a RFC-0152 (três teoremas de entrada
fixa + dente já existiam em `TxGlue.lean`), mas faltava o ∀. A product
guarantee: **TX falho commitado nunca fica vivo na maioria** — a
maioria reverta exatamente quando o TX falhou e o range commitou.

## O que já corria no rustc (fatia de graduação — sem edit de código)

```rust
// crates/pedradb-store/src/tx_glue_kernel.rs
pub enum TxRangeAction { KeepCommitted, LocalRevert, MajorityRevert }
pub fn tx_range_action(range_committed: bool, tx_failed: bool)
    -> TxRangeAction
```

Trampolim `tx_finish` (lib.rs:8090) já casa no kernel; planta
`tx_glue_kernel::tests::tx_range_action_on_live_majority_is_not_ok`
já existe. Primeiro fire pago fora de txn_kernel: o extrato
`TxGlueKernel.lean` (aeneas_tx_glue.sh) e o wrapper `TxGlue.lean` já
existiam — a graduação não precisou de extrato novo.

## Teorema (atom registrado)

`tx_range_action_majority_reverts_iff_failed_and_committed`
(`TxGlue.lean`, ∀ `range_committed tx_failed`): plano =
`MajorityRevert` ↔ (`tx_failed = true` ∧ `range_committed = true`).
0 sorry, de primeira. Subsume os três teoremas de entrada fixa. AS-IS
dente `tx_range_action_as_is_local_only` (committed+failed só reverte
local — a maioria mantém a entrada envenenada) já registrado.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 125 | **124** (`tx_glue` gradua) |
| `floor_atom` | 6 | **7** (linha `atom catalog:tx_glue`) |
| `floor_extract` / residuals `extract` | 273 | **272** (recount: par promovido sai do pool) |
| residuals `proof_depth` | 273/2/6/17 | **272/2/7/17** |
| residuals `data_fate`/`single_artifact` | 125/291 | **124/291** |

## Evidência

- `lake build TxGlue` → verde, 0 sorry.
- `cargo test -p pedradb-store --lib tx_range_action` → planta verde.
- depth-floor GREEN (extract 272, atom 7/7, data_fate 124≤124);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.

## Próximos candidatos (análise feita neste fire)

- `compact_unleft` (store compact_kernel, extrato + wrapper existem):
  iff direta é FALSA — degenerescência numérica (j = through+1 no
  braço else devolve `ok through = ok (j−1)`); formular via ≠ precisa
  de lemas U64 (saturating_sub <). Mais caro.
- Ramo `Batch` do `apply_range` (lib.rs:7128): shape (a) — kernel
  novo reusando `apply_put_plan` por chave; precisa par novo + trampolim.
- `revert_clears_status`: payload `¬b` no extrato — elaboração
  arriscada, só com statement de sintaxe de superfície copiada.
