# RFC-0191 P2.3 (passo 7) — revert_clears_status graduado a atom; meta atom 8/8 fechada (cap 124→123)

**Data:** 2026-09-10 · **Fire:** 765 · **Estado:** landed

## Alvo

O par `revert_clears_status` (F47, `txn_kernel.rs`) estava parado em
`data_fate`. A product guarantee: **a cerca de abort sobrevive** — a
chave de status do txn só pode cair depois de um revert quando todos os
pares foram embora E o status não era abort (um `TxnCommit` em replay
ainda vê o abort).

## Teorema (atom registrado)

`revert_clears_status_clears_iff_pairs_gone_and_live` (`Txn.lean`, ∀
`status_is_abort pairs_empty`): limpa ↔ (`pairs_empty = true` ∧
`status_is_abort = false`). 0 sorry. O medo da elaboração do payload
`¬b` (extrato: `ok (¬ status_is_abort)`) não se confirmou: `simp at h`
resolve a igualdade `ok (¬s) = ok true` direto para `s = false`.

### Lean lore (Fire 765)

- Payload `¬b` do extrato NÃO é obstáculo: no ramo then do forward,
  `simp at h` fecha `ok (¬s) = ok true → s = false`; backward
  `rw [if_pos hp, ha]; rfl` (com `ha : s = false`, `¬false` reduz).
- Ler o NÚMERO da linha de erro antes de editar: "simp made no
  progress" na 159 era o `simp only [ok]` (que nunca progressa),
  não o `simp at h` — removi a linha errada na primeira tentativa.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 124 | **123** (`revert_clears_status` gradua) |
| `floor_atom` | 7 | **8 — meta atom do P2.3 fechada** |
| `floor_extract` / residuals `extract` | 272 | **271** (recount) |
| residuals `proof_depth` | 272/2/7/17 | **271/2/8/17** |
| residuals `data_fate`/`single_artifact` | 124/291 | **123/291** |

## Evidência

- `lake build Txn` → verde, 0 sorry (2 iterações de tactics).
- `cargo test -p pedradb-store --lib revert_clears_status` → planta
  `three_teeth_queued::revert_clears_status_on_live_queued_is_not_ok`
  verde.
- depth-floor GREEN (extract 271, atom 8/8, data_fate 123≤123);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.

## Estado da cadência P2.3

- Atoms: **8/8 — fechado.**
- Cap: 123 → falta descer até ≤100 (22 descidas), shape (a) novo
  kernel + par graduado por commit, ou mais graduações do pool amplo
  (fora do store: membership, raft, fdb-recipes — `sched_plant_joint`
  é o próximo candidato com extrato).
