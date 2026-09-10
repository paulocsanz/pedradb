# RFC-0191 P2.3 (passo 4) — revert de usuário graduado a atom (cap 127→126)

**Data:** 2026-09-10 · **Fire:** 762 · **Estado:** landed

## Alvo

O par `revert_user_action` (F34, `txn_kernel.rs`) estava parado em
`data_fate`: o kernel decide o destino de cada chave de usuário no
`apply_txn_revert` desde a RFC-0152, mas a decisão de 3 braços (restaurar
valor / restaurar ausência / não tocar) não tinha ∀ registrado. A
product guarantee por trás: **revert nunca dá delete às cegas** — um
registro de pré-imagem ausente não significa que a chave não existia
(`LeaveUntouched`, não `RestoreAbsent`).

## O que já corria no rustc (fatia de graduação — sem edit de código)

```rust
// crates/pedradb-store/src/txn_kernel.rs
pub enum RevertUserAction { RestoreValue, RestoreAbsent, LeaveUntouched }
pub fn revert_user_action(had_pre_record: bool, pre_was_absent: bool)
    -> RevertUserAction
```

O trampolim `apply_txn_revert` (lib.rs) já casa no kernel; a planta
`revert_user_action_on_live_queued_is_not_ok` (three_teeth_queued.rs) já
existe. Este fire paga o degrau de prova, não um novo extrato.

## Teorema (atom registrado)

`revert_user_action_restore_value_iff_record_and_present` (`Txn.lean`,
∀ `had_pre_record pre_was_absent`): plano = `RestoreValue` ↔
(`had_pre_record = true` ∧ `pre_was_absent = false`). 0 sorry. AS-IS
dente `revert_user_action_as_is` sempre `RestoreAbsent` (delete às cegas
de chave que o peer nunca preparou — F34).

### Lean lore (Fire 762)

- Forward com negação no braço: `next c2` traz `¬(pre_was_absent =
  true)`; a ponte para `pre_was_absent = false` é `by simp at c2;
  exact c2` (`Bool.not_eq_true` como simp lemma).
- Backward: `rw [if_pos hd, if_neg (by simp [hp])]` fecha sozinho
  (if_neg alimentado por `simp [hp]` com `hp : x = false` provando
  `¬(x = true)`).
- Padrão de colagem: prefixo de número de linha do `read_file` DENTRO do
  arquivo vira `100 · intro h` → erro de parse "unexpected token"; o
  relatório mostra mp/mpr "unsolved" porque o bloco inteiro depois do
  `constructor` não anexa.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 127 | **126** (`revert_user_action` gradua) |
| `floor_atom` | 4 | **5** (linha `atom catalog:revert_user_action`) |
| `floor_extract` / residuals `extract` | 275 | **274** (recount: o par promovido a atom sai do pool extract — mesma classe do 276→275 do P1.5, não extração perdida) |
| residuals `proof_depth` | 275/2/4/17 | **274/2/5/17** |
| residuals `data_fate`/`single_artifact` | 127/291 | **126/291** |

## Evidência

- `lake build Txn` → verde, 0 sorry.
- `cargo test -p pedradb-store revert_user_action` → planta verde.
- depth-floor GREEN (atom=5/floor 5, data_fate 126≤126); product-floor
  GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.
