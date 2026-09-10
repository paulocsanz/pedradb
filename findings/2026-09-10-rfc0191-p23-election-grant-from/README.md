# RFC-0191 P2.3 (passo 10) — election_grant_from graduado a atom (cap 121→120)

**Data:** 2026-09-10 · **Fire:** 768 · **Estado:** landed

## Alvo

O par `election_grant_from` (RFC-0114/0116,
`pedradb-raft/src/membership_kernel.rs`, entry `election_grant_from_counts`)
estava parado em `data_fate`. A garantia: **o voto concedido por um nó
conta exatamente quando o nó está no conjunto de ids, ou está no pending
old-or-new do joint** — nó fora dos dois conjuntos nunca concede. Terceira
graduação em Membership.lean.

## Teorema (atom registrado)

`election_grant_from_counts_ok_iff_ids_or_pending` (`Membership.lean`,
∀ `in_ids in_pending_old_or_new`): `ok true` ↔
(`in_ids = true` ∨ `in_pending_old_or_new = true`). 0 sorry, verde na
primeira build — a lore do Fire 767 (disjunção de equações; `split`+
`rw` no backward com condição desconhecida) transferiu direto.

## Contabilidade (mesmo commit)

| chave | antes | depois |
|---|---|---|
| `cap_data_fate` | 121 | **120** (`election_grant_from` gradua) |
| `floor_atom` | 10 | **11** |
| `floor_extract` / residuals `extract` | 269 | **268** (recount) |
| residuals `proof_depth` | 269/2/10/17 | **268/2/11/17** |
| residuals `data_fate`/`single_artifact` | 121/291 | **120/291** |

## Evidência

- `lake build Membership` → verde, 0 sorry (warnings só na Aeneas.Std).
- `cargo test -p pedradb-store --lib
  election_grant_from_counts_on_live_queued` → planta
  `three_teeth_queued::election_grant_from_counts_on_live_queued_is_not_ok`
  verde.
- depth-floor GREEN (extract 268, atom 11/11, data_fate 120≤120);
  product-floor GREEN (promoted 4).
- Sem regen de extrato: nenhum arquivo Rust mudou neste fire.

## Descartes neste fire (pool Membership)

- `pending_joint_node_counts` = `ok is_member` — wrap-class, nunca ganha.
- `joint_leave_ok` = `ok leave_in_log` — wrap-class, nunca ganha.
