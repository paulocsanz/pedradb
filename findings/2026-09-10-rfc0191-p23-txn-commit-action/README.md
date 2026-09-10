# RFC-0191 P2.3 (passo 5) — commit-vs-revert do txn graduado a atom (cap 126→125)

**Data:** 2026-09-10 · **Fire:** 763 · **Estado:** landed

## Alvo

O par `txn` (F47/F34/F52/F35/F36/F49/F50, `txn_kernel.rs`, entry
`txn_commit_action`) estava parado em `data_fate`. O kernel decide commit
ou revert pelo status do txn desde a RFC-0152 (o teorema fixo
`txn_commit_action_abort_reverts` já existia em `Txn.lean`), mas faltava
o ∀. A product guarantee: **commit de txn abortado sempre reverta e
nunca materializa** — a cerca de abort (fence) sempre ganha (F47).

## O que já corria no rustc (fatia de graduação — sem edit de código)

```rust
// crates/pedradb-store/src/txn_kernel.rs
pub enum TxnCommitAction { Materialise, Revert }
pub fn txn_commit_action(status_is_abort: bool) -> TxnCommitAction
```

Trampolim `apply_txn_commit` (lib.rs:1466) já casa no kernel; planta
`txn_kernel::three_teeth::txn_commit_action_on_live_abort_is_not_ok`
já existe. Este fire paga o degrau de prova.

## Teorema (atom registrado)

`txn_commit_action_reverts_iff_abort` (`Txn.lean`, ∀
`status_is_abort`): plano = `Revert` ↔ (`status_is_abort = true`). 0
sorry. Subsume o teorema fixo `txn_commit_action_abort_reverts`. AS-IS
dente `txn_commit_action_as_is` sempre `Materialise` (abort materializa
— F47), já registrado.

### Lean lore (Fire 763)

- If único de construtores distintos é o átomo mais barato: forward
  `split at h` + `exact c1` / `absurd h (by simp)`; backward
  `rw [if_pos hd]` fecha sozinho.
- Armadilha de exibição: `tail -1` do TSV não renderiza tabs no
  terminal do harness — validar com `sed -n l` antes de "consertar"
  linha que já está certa.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 126 | **125** (`txn` gradua) |
| `floor_atom` | 5 | **6** (linha `atom catalog:txn`) |
| `floor_extract` / residuals `extract` | 274 | **273** (recount: par promovido sai do pool) |
| residuals `proof_depth` | 274/2/5/17 | **273/2/6/17** |
| residuals `data_fate`/`single_artifact` | 126/291 | **125/291** |

## Evidência

- `lake build Txn` → verde, 0 sorry.
- `cargo test -p pedradb-store --lib txn_commit_action` → planta verde.
- depth-floor GREEN (extract 273, atom 6/6, data_fate 125≤125);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
