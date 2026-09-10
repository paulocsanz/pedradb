# RFC-0191 P2.3 (passo 11) — joint_election graduado a atom (cap 120→119)

**Data:** 2026-09-10 · **Fire:** 769 · **Estado:** landed

## Alvo

O par `joint_election` (F-L28/RFC-0064, `pedradb-raft/src/
membership_kernel.rs`, entry `joint_election_ok`) estava parado em
`data_fate`. A garantia: **a eleição conjunta elege exatamente quando
C-old tem maioria E (não há joint pendente, ou C-new também tem
maioria)** — um lado sozinho nunca elege. Último if real do pool
Membership (29 pares restantes varridos: só este tinha if; os demais
são wrap-class `ok x` / `ok (¬x)` / `ok (x > y)`).

## Teorema (atom registrado)

`joint_election_ok_elects_iff_old_and_new_majority` (`Membership.lean`,
∀ `old_yes old_n new_yes : Option`): `ok true` ↔
`((old_yes >= maj old_n) && (match new_yes with | none => true |
some p => p.1 >= maj p.2)) = true`. Prova de 2 linhas: `rw
[c1_joint_election]` (o close registrado no P1.4 já dá a forma fechada
∀ contagens+Option) + `simp` (`ok.injEq` + `iff_refl`). 0 sorry, verde
na primeira build.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 120 | **119** (`joint_election` gradua) |
| `floor_atom` | 11 | **12** |
| `floor_extract` / residuals `extract` | 268 | **267** (recount) |
| residuals `proof_depth` | 268/2/11/17 | **267/2/12/17** |
| residuals `data_fate`/`single_artifact` | 120/291 | **119/291** |

## Evidência

- `lake build Membership` → verde, 0 sorry (warnings só na Aeneas.Std).
- `cargo test -p pedradb-store --lib joint_election_ok_on_live_queued`
  → planta verde.
- depth-floor GREEN (extract 267, atom 12/12, data_fate 119≤119);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.

## Pool Membership esgotado para graduação

Varredura completa dos 29 `data_fate` restantes do wrapper
`aeneas_membership.sh`: nenhum outro if — todos wrap-class (ex. `ok
is_local`, `ok (¬ in_ids)`, `ok (commit > applied)`, `Ord.max`,
`Slice ≠`). Próximos pagamentos vêm de outros wrappers (Cf.lean 7,
core compact/recover/manifest/reopen/vlog/batch/merge/locktab/iter/
leveling, raft ae/commit/vote).
